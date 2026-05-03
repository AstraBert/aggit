use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct S3Origin {
    endpoint: String,
    secret_key: String,
    key_id: String,
}

const ORIGIN_FILE: &str = ".aggitorigin";
const GITIGNORE: &str = ".gitignore";
const AGGITIGNORE: &str = ".aggitignore";

pub fn add_origin_to_gitignore() -> anyhow::Result<()> {
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

pub fn create_origin(
    name: &str,
    endpoint: String,
    secret_key: String,
    key_id: String,
) -> anyhow::Result<()> {
    let origin = S3Origin {
        endpoint,
        secret_key,
        key_id,
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

pub fn update_config(
    name: &str,
    endpoint: Option<String>,
    secret_key: Option<String>,
    key_id: Option<String>,
) {
}
