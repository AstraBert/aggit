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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_aggit_dir() {
        fs::create_dir_all(".aggit").unwrap();
    }

    fn cleanup_aggit_dir() {
        let _ = fs::remove_dir_all(".aggit");
    }

    #[test]
    #[serial_test::serial]
    fn test_config_repository_success() {
        cleanup_aggit_dir();
        setup_aggit_dir();

        let result = config_repository(
            "test-repo".to_string(),
            "A test repository".to_string(),
            vec!["test".to_string(), "rust".to_string()],
        );
        assert!(result.is_ok());
        assert!(PathBuf::from(".aggit/repo.toml").exists());

        let content = fs::read_to_string(".aggit/repo.toml").unwrap();
        assert!(content.contains("name = \"test-repo\""));
        assert!(content.contains("description = \"A test repository\""));

        cleanup_aggit_dir();
    }

    #[test]
    #[serial_test::serial]
    fn test_config_repository_already_configured() {
        cleanup_aggit_dir();
        setup_aggit_dir();

        config_repository(
            "test-repo".to_string(),
            "A test repository".to_string(),
            vec!["test".to_string()],
        )
        .unwrap();

        let result = config_repository(
            "test-repo-2".to_string(),
            "Another test repository".to_string(),
            vec!["test".to_string()],
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("already configured"));

        cleanup_aggit_dir();
    }

    #[test]
    #[serial_test::serial]
    fn test_get_repository_success() {
        cleanup_aggit_dir();
        setup_aggit_dir();

        config_repository(
            "my-repo".to_string(),
            "My description".to_string(),
            vec!["a".to_string(), "b".to_string()],
        )
        .unwrap();

        let repo = get_repository().unwrap();
        assert_eq!(repo.name, "my-repo");
        assert_eq!(repo.description, "My description");
        assert_eq!(repo.topics, vec!["a", "b"]);

        cleanup_aggit_dir();
    }

    #[test]
    #[serial_test::serial]
    fn test_get_repository_not_configured() {
        cleanup_aggit_dir();
        setup_aggit_dir();

        let result = get_repository();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("yet to be configured"));

        cleanup_aggit_dir();
    }
}
