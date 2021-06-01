use serde_derive::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub repo_root: Option<PathBuf>,

    pub managed_repos: Option<Vec<RepoConfig>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RepoConfig {
    pub path: PathBuf,
}
