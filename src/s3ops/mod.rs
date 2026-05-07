use std::{collections::HashMap, fs, io::Read, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::anyhow;
use aws_config::{BehaviorVersion, Region, retry::RetryConfig};
use aws_sdk_s3::{
    Client, config::Credentials, operation::get_object::GetObjectError, primitives::ByteStream,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    gitops::{
        collect_reachable_objects, find_object, get_current_branch, get_local_current_hash,
        index_path_for_branch, read_object, restore_working_tree,
    },
    repository::get_repository,
};

const ORIGIN_FILE: &str = ".aggitorigin";
const GITIGNORE: &str = ".gitignore";
const AGGITIGNORE: &str = ".aggitignore";
const MAX_CONCURRENT_UPLOADS: usize = 10;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct S3Origin {
    endpoint: String,
    secret_key: String,
    key_id: String,
    region: String,
}

#[derive(Debug)]
enum OriginAction {
    Create,
    Update,
    Add,
}

impl FromStr for OriginAction {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "add" => Ok(Self::Add),
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            _ => anyhow::bail!("unknown action: {}", s),
        }
    }
}

fn add_origin_to_gitignore() -> anyhow::Result<()> {
    let mut gitignore = fs::read_to_string(GITIGNORE)?;
    let mut aggitignore = fs::read_to_string(AGGITIGNORE)?;
    if !gitignore.contains(ORIGIN_FILE) {
        gitignore = format!("{}\n{}\n", gitignore, ORIGIN_FILE);
        fs::write(GITIGNORE, gitignore)?;
    }
    if !aggitignore.contains(ORIGIN_FILE) {
        aggitignore = format!("{}\n{}\n", aggitignore, ORIGIN_FILE);
        fs::write(AGGITIGNORE, aggitignore)?;
    }
    Ok(())
}

fn create_origin(
    name: &str,
    endpoint: String,
    secret_key: String,
    key_id: String,
    region: String,
) -> anyhow::Result<()> {
    let origin = S3Origin {
        endpoint,
        secret_key,
        key_id,
        region,
    };
    let mut origins = HashMap::new();
    origins.insert(name, origin);
    let origin_path = PathBuf::from(ORIGIN_FILE);
    if origin_path.exists() {
        return Err(anyhow!(
            "{} already exists. If you want to modify or add a new origin, use the `update` or `add` actions.",
            ORIGIN_FILE
        ));
    }
    let origin_str = toml::to_string(&origins)?;
    fs::write(&origin_path, origin_str)?;
    add_origin_to_gitignore()?;
    Ok(())
}

fn update_origin(
    name: &str,
    endpoint: Option<String>,
    secret_key: Option<String>,
    key_id: Option<String>,
    region: Option<String>,
) -> anyhow::Result<()> {
    let origin_path = PathBuf::from(ORIGIN_FILE);
    if !origin_path.exists() {
        return Err(anyhow!(
            "{} does not exist, use the `create` action to add your first origin",
            ORIGIN_FILE
        ));
    }
    let content = fs::read_to_string(&origin_path)?;
    let mut validated: HashMap<String, S3Origin> = toml::from_str(&content)?;
    if !validated.contains_key(name) {
        return Err(anyhow!("Origin {} does not exist", name));
    }
    validated.entry(name.to_owned()).and_modify(|v| {
        if let Some(e) = endpoint {
            v.endpoint = e;
        }
        if let Some(sk) = secret_key {
            v.secret_key = sk;
        }
        if let Some(ki) = key_id {
            v.key_id = ki;
        }
        if let Some(r) = region {
            v.region = r;
        }
    });
    let updated = toml::to_string(&validated)?;
    fs::write(&origin_path, updated)?;

    Ok(())
}

pub fn add_origin(
    name: &str,
    endpoint: String,
    secret_key: String,
    key_id: String,
    region: String,
) -> anyhow::Result<()> {
    let origin_path = PathBuf::from(ORIGIN_FILE);
    if !origin_path.exists() {
        return Err(anyhow!(
            "{} does not exist, use the `create` action to add your first origin",
            ORIGIN_FILE
        ));
    }
    let content = fs::read_to_string(&origin_path)?;
    let mut validated: HashMap<String, S3Origin> = toml::from_str(&content)?;
    if validated.contains_key(name) {
        return Err(anyhow!(
            "Origin {} already exists, use the `update` action to update it",
            name
        ));
    }
    validated.insert(
        name.to_string(),
        S3Origin {
            endpoint,
            secret_key,
            key_id,
            region,
        },
    );
    let added = toml::to_string(&validated)?;
    fs::write(&origin_path, added)?;

    Ok(())
}

pub fn manage_origin(
    action: &str,
    name: &str,
    endpoint: Option<String>,
    secret_key: Option<String>,
    key_id: Option<String>,
    region: Option<String>,
) -> anyhow::Result<()> {
    let validated_action = OriginAction::from_str(action)?;
    match validated_action {
        OriginAction::Create => {
            if vec![&endpoint, &secret_key, &key_id, &region]
                .iter()
                .any(|x| x.is_none())
            {
                return Err(anyhow!(
                    "Endpoint, secret key and key ID are all required to create an S3 origin"
                ));
            }
            create_origin(
                name,
                endpoint.unwrap(),
                secret_key.unwrap(),
                key_id.unwrap(),
                region.unwrap(),
            )?;
            println!("Successfully created origin {} at {}", name, ORIGIN_FILE);
        }
        OriginAction::Update => {
            update_origin(name, endpoint, secret_key, key_id, region)?;
            println!("Successfully updated origin {} in {}", name, ORIGIN_FILE);
        }
        OriginAction::Add => {
            if vec![&endpoint, &secret_key, &key_id, &region]
                .iter()
                .any(|x| x.is_none())
            {
                return Err(anyhow!(
                    "Endpoint, secret key and key ID are all required to add an S3 origin to existing origins"
                ));
            }
            add_origin(
                name,
                endpoint.unwrap(),
                secret_key.unwrap(),
                key_id.unwrap(),
                region.unwrap(),
            )?;
            println!(
                "Successfully added origin {} to existing origins in {}",
                name, ORIGIN_FILE
            );
        }
    }

    Ok(())
}

fn get_origin(origin: &str) -> anyhow::Result<S3Origin> {
    let content = fs::read_to_string(ORIGIN_FILE)?;
    let validated: HashMap<String, S3Origin> = toml::from_str(&content)?;
    if !validated.contains_key(origin) {
        return Err(anyhow!("No such origin: {}", origin));
    }
    let ori = validated.get(origin).unwrap();
    Ok(ori.clone())
}

async fn get_client(ori: S3Origin) -> Arc<Client> {
    let credentials = Credentials::new(ori.key_id, ori.secret_key, None, None, "s3");
    let region = Region::new(ori.region);
    let shard_config = aws_config::defaults(BehaviorVersion::latest())
        .region(region)
        .credentials_provider(credentials)
        .endpoint_url(ori.endpoint)
        .retry_config(RetryConfig::standard())
        .load()
        .await;
    let client = Client::new(&shard_config);

    Arc::new(client)
}

async fn check_or_create_bucket(origin: &str, client: &Arc<Client>) -> anyhow::Result<String> {
    let repository = get_repository()?;
    let main_branch = get_current_branch()?;
    // check if origin exists
    let _ = get_origin(origin)?;
    let bucket_name = format!("{}-{}-{}", origin, repository.name, main_branch);
    let buckets_list = client
        .list_buckets()
        .prefix(&bucket_name)
        .max_buckets(1)
        .send()
        .await?;
    let buckets = buckets_list.buckets();
    if buckets.is_empty() {
        println!(
            "↻ Bucket for origin {}, repository {} and branch {} does not exist, creating it...",
            origin, repository.name, main_branch
        );
        client.create_bucket().bucket(&bucket_name).send().await?;
        println!("✔ Bucket successfully created");
    }
    println!(
        "✔ Bucket for origin {}, repository {} and branch {} exists",
        origin, repository.name, main_branch
    );

    return Ok(bucket_name);
}

async fn get_remote_head(
    bucket_name: &str,
    client: &Arc<Client>,
) -> anyhow::Result<Option<String>> {
    let result = client
        .get_object()
        .bucket(bucket_name)
        .key("head")
        .send()
        .await;
    match result {
        Ok(o) => {
            let bts = o.body.collect().await?.into_bytes();
            let mut head = String::new();
            let mut chars = bts.iter().as_slice();
            chars.read_to_string(&mut head)?;
            Ok(Some(head))
        }
        Err(e) => {
            let service_err = e.into_service_error();
            match service_err {
                GetObjectError::NoSuchKey(_) => Ok(None),
                _ => Err(anyhow!(service_err.to_string())),
            }
        }
    }
}

async fn create_object(
    key: &str,
    content: Vec<u8>,
    bucket_name: &str,
    client: Arc<Client>,
) -> anyhow::Result<()> {
    client
        .put_object()
        .bucket(bucket_name)
        .key(key)
        .body(ByteStream::from(content))
        .send()
        .await?;
    Ok(())
}

async fn download_object(
    bucket_name: &str,
    key: &str,
    client: Arc<Client>,
) -> anyhow::Result<(String, Vec<u8>)> {
    let resp = client
        .get_object()
        .bucket(bucket_name)
        .key(key)
        .send()
        .await?;
    let content = resp
        .body
        .collect()
        .await?
        .into_bytes()
        .iter()
        .as_slice()
        .to_vec();

    Ok((format!(".aggit/{}", key), content))
}

fn trim_path(path: &PathBuf) -> anyhow::Result<String> {
    let path_str = path.to_str().ok_or(anyhow!("Non-UTF8 path"))?;
    path_str
        .split_once(".aggit/")
        .map(|(_, after)| after.to_owned())
        .ok_or(anyhow!("Path does not contain .aggit/: {}", path_str))
}

async fn upload_objects_and_head(
    remote_head: Option<&str>,
    bucket_name: String,
    client: Arc<Client>,
) -> anyhow::Result<i32> {
    let (current_head, _) = get_local_current_hash()?;
    let current_head = current_head.ok_or(anyhow!("No current head, nothing to push"))?;
    let objects = collect_reachable_objects(&current_head, remote_head)?;
    let mut object_content = HashMap::new();
    for obj in objects {
        let path = find_object(&obj)?;
        let (_, content) = read_object(&obj)?;
        object_content.insert(trim_path(&path)?, content);
    }
    object_content.insert("head".to_string(), current_head.into_bytes());
    let index_path = index_path_for_branch(&get_current_branch()?);
    let index_content = fs::read(&index_path)?;
    object_content.insert(trim_path(&index_path)?, index_content);
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_UPLOADS));
    let mut join_set = JoinSet::new();
    for (k, v) in object_content {
        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let bucket_name = bucket_name.clone();
        join_set.spawn(async move {
            let _permit = permit;
            create_object(&k, v, &bucket_name, client).await
        });
    }

    let mut failed = 0;
    let mut panicked = 0;
    let mut success = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {
                success += 1;
            }
            Ok(Err(e)) => {
                eprintln!("Error while uploading the object: {e}");
                failed += 1;
            }
            Err(e) => {
                eprintln!("Error while executing object upload function: {e}");
                panicked += 1;
            } // JoinError
        }
    }

    if failed > 0 || panicked > 0 {
        return Err(anyhow!("{} uploads failed, {} panicked", failed, panicked));
    }

    Ok(success)
}

pub async fn push(origin: String) -> anyhow::Result<()> {
    let ori = get_origin(&origin)?;
    let client = get_client(ori).await;
    let bucket_name = check_or_create_bucket(&origin, &client).await?;
    let remote_head = get_remote_head(&bucket_name, &client).await?;
    let success = upload_objects_and_head(remote_head.as_deref(), bucket_name, client).await?;
    println!("Successfully pushed {:?} objects", success);

    Ok(())
}

pub async fn clone(
    origin: String,
    repository_name: String,
    branch: Option<String>,
) -> anyhow::Result<()> {
    let branch = branch.unwrap_or("main".to_string());
    let ori = get_origin(&origin)?;
    let client = get_client(ori).await;
    let bucket_name = format!("{}-{}-{}", origin, repository_name, branch);
    let all_objects = client.list_objects().bucket(&bucket_name).send().await?;
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_UPLOADS));
    let mut join_set: JoinSet<Result<(), anyhow::Error>> = JoinSet::new();
    if let Some(objs) = all_objects.contents {
        for o in objs {
            if let Some(key) = o.key {
                let permit = semaphore.clone().acquire_owned().await?;
                let client = client.clone();
                let bucket_name = bucket_name.clone();
                join_set.spawn(async move {
                    let _permit = permit;
                    let (path, contents) = download_object(&bucket_name, &key, client).await?;
                    fs::write(path, contents).map_err(|e| anyhow!(e.to_string()))
                });
            }
        }
    }

    let mut failed = 0;
    let mut panicked = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("Error while uploading the object: {e}");
                failed += 1;
            }
            Err(e) => {
                eprintln!("Error while executing object upload function: {e}");
                panicked += 1;
            } // JoinError
        }
    }

    if failed > 0 || panicked > 0 {
        return Err(anyhow!("{} uploads failed, {} panicked", failed, panicked));
    }

    println!("Successfully written the .aggit/ directory");

    restore_working_tree(&branch)?;

    println!("Successfully cloned {} from {}", repository_name, origin);

    Ok(())
}
