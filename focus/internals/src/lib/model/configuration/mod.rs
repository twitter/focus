// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

mod index_config;
mod notification_config;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

pub use index_config::IndexConfig;
pub use notification_config::NotificationCategory;
pub use notification_config::NotificationConfig;

use super::persistence;

pub const INDEX_CONFIG_FILENAME: &str = "index.cfg.json";
pub const NOTIFICATION_CONFIG_FILENAME: &str = "notification.cfg.json";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    pub index: IndexConfig,
    pub notification: NotificationConfig,
}

impl Configuration {
    pub fn new(repo_path: impl AsRef<Path>) -> anyhow::Result<Configuration> {
        let config_dir = Self::config_dir(repo_path);
        if !config_dir.is_dir() {
            return Ok(Default::default());
        }

        Ok(Self {
            index: Self::load(&config_dir, INDEX_CONFIG_FILENAME),
            notification: Self::load(&config_dir, NOTIFICATION_CONFIG_FILENAME),
        })
    }

    pub fn config_dir(repo_path: impl AsRef<Path>) -> PathBuf {
        repo_path.as_ref().join(".focus").join("config")
    }

    fn load<T: Default + DeserializeOwned>(config_dir: impl AsRef<Path>, name: &str) -> T {
        let config_dir = config_dir.as_ref();
        persistence::load_model(config_dir.join(name)).unwrap_or_default()
    }
}

#[cfg(test)]
mod testing {
    use maplit::btreeset;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn provides_defaults_in_case_of_nonexistent_config() {
        let dir = tempdir().unwrap();
        assert_eq!(Configuration::new(dir.path()).unwrap(), Default::default());
    }

    #[test]
    fn reading_index_config() {
        let dir = tempdir().unwrap();
        let remote_dir = dir.path().join("index_remote");
        let repo_dir = dir.path().join("repo");
        let config_dir = Configuration::config_dir(&repo_dir);
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let index_config_path = config_dir.join(INDEX_CONFIG_FILENAME);
        let in_memory_config = IndexConfig {
            enabled: true,
            remote: format!("file://{}", remote_dir.display()),
        };
        persistence::store_model(&index_config_path, &in_memory_config).unwrap();
        assert_eq!(
            Configuration::new(&repo_dir).unwrap().index,
            in_memory_config
        );
    }

    #[test]
    fn reading_notification_config() {
        let dir = tempdir().unwrap();
        let remote_dir = dir.path().join("index_remote");
        let repo_dir = dir.path().join("repo");
        let config_dir = Configuration::config_dir(&repo_dir);
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let index_config_path = config_dir.join(NOTIFICATION_CONFIG_FILENAME);
        let in_memory_config = NotificationConfig {
            suppressed_categories: btreeset![NotificationCategory::BuildGraphState],
        };
        persistence::store_model(&index_config_path, &in_memory_config).unwrap();
        assert_eq!(
            Configuration::new(&repo_dir).unwrap().notification,
            in_memory_config
        );
        assert!(!in_memory_config.is_allowed(NotificationCategory::BuildGraphState));
    }

    #[test]
    fn notification_config_defaults_to_unsupressed() {
        let config = NotificationConfig::default();
        assert!(config.suppressed_categories.is_empty());
        assert!(config.is_allowed(NotificationCategory::BuildGraphState));
    }
}
