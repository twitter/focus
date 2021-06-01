use std::collections::HashSet;
use std::path::{Path, PathBuf};

static APP_ID: &str = "biz.twitter.focus";
static CONFIG_DIR_KEY: &str = "CONFIG_DIR";
static CACHE_DIR_KEY: &str = "CACHE_DIR";
static DATA_DIR_KEY: &str = "DATA_DIR";
static REPO_DIR_KEY: &str = "REPO_DIR";

pub fn config_dir() -> PathBuf {
    match std::env::var(CONFIG_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::preference_dir().unwrap().join(APP_ID).to_path_buf(),
    }
}

pub fn cache_dir() -> PathBuf {
    match std::env::var(CACHE_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::cache_dir().unwrap().join(APP_ID).to_path_buf(),
    }
}

pub fn data_dir() -> PathBuf {
    match std::env::var(DATA_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::data_dir().unwrap().join(APP_ID).to_path_buf(),
    }
}

pub fn workspace_dir() -> PathBuf {
    match std::env::var(REPO_DIR_KEY) {
        Ok(val) => Path::new(&val).to_path_buf(),
        _ => dirs::home_dir().unwrap().join("workspace").to_path_buf(),
    }
}
