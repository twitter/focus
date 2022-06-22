// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

mod index_config;
use std::path::{Path, PathBuf};

pub use index_config::IndexConfig;

use super::persistence;

pub const INDEX_CONFIG_FILENAME: &str = "index.cfg.json";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    pub index: IndexConfig,
}

impl Configuration {
    pub fn new(repo_path: impl AsRef<Path>) -> anyhow::Result<Configuration> {
        let config_dir = Self::config_dir(repo_path);
        if !config_dir.is_dir() {
            return Ok(Default::default());
        }

        Ok(Self {
            index: persistence::load_model(config_dir.join(INDEX_CONFIG_FILENAME))
                .unwrap_or_default(),
        })
    }

    pub fn config_dir(repo_path: impl AsRef<Path>) -> PathBuf {
        repo_path.as_ref().join(".focus").join("config")
    }
}

#[cfg(test)]
mod testing {
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
}
