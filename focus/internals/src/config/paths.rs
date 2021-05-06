use std::collections::HashSet;
use std::path::{Path, PathBuf};

static APP_ID: &str = "biz.twitter.focus";
static CONFIG_DIR_KEY: &str = "CONFIG_DIR";
static CACHE_DIR_KEY: &str = "CACHE_DIR";

pub fn config_dir() -> PathBuf {
    match std::env::var(CONFIG_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::preference_dir().unwrap().join(APP_ID).to_path_buf(),
    }
}

pub fn cache_dir() -> PathBuf {
    match std::env::var(CACHE_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::cache_dir()
            .unwrap()
            .join(APP_ID)
            .to_path_buf()
            .to_path_buf(),
    }
}

pub fn default_repo_root() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join("workspace")
        .join("repos")
        .to_path_buf()
}

pub fn default_source_root() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join("workspace")
        .join("source")
        .to_path_buf()
}

pub fn default_target_extractors() -> HashSet<String> {
    let mut set = HashSet::<String>::new();
    set.insert(String::from("bazel"));
    set.insert(String::from("pants"));
    return set;
}

pub fn default_authorities() -> Vec<super::structures::RepoConfig> {
    vec![super::structures::RepoConfig {
        path: default_source_root(),
        target_extractors: default_target_extractors(),
    }]
}
