use std::{fs, path::PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Repository {
    pub name: String,
    pub description: String,
    pub topics: Vec<String>,
}

pub fn config_repository(
    name: String,
    description: String,
    topics: Vec<String>,
) -> anyhow::Result<()> {
    let repository_path = PathBuf::from(".aggit").join("repo.toml");
    if repository_path.exists() {
        return Err(anyhow!("Repository had been already configured"));
    }
    let repo = Repository {
        name,
        description,
        topics,
    };
    let repo_str = toml::to_string(&repo)?;
    fs::write(repository_path, repo_str)?;
    println!("Successfully configured repository");
    Ok(())
}

pub fn get_repository() -> anyhow::Result<Repository> {
    let repository_path = PathBuf::from(".aggit").join("repo.toml");
    if !repository_path.exists() {
        return Err(anyhow!("Repository is yet to be configured"));
    }
    let repo_str = fs::read_to_string(repository_path)?;
    let validated: Repository = toml::from_str(&repo_str)?;

    Ok(validated)
}
