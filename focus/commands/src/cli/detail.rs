use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use git2::Repository;
use internals::error::AppError;
use log::error;
use std::ffi::OsString;

lazy_static! {
    static ref BARE_REPO_EXTENSION: OsString = OsString::from("git");
}

#[allow(dead_code)]
pub(crate) struct Repo {
    repo: Repository,
}

impl Repo {
    #[allow(dead_code)]
    fn new(path: &Path) -> Result<Self> {
        Repository::open(path)
            .map(|repo| Self { repo })
            .map_err(|err| err.into())
    }
}

#[allow(dead_code)]
pub fn server(_repo: &Path, _data: &Path) -> Result<(), AppError> {
    todo!("impl")
}

#[allow(dead_code)]
pub(crate) fn find_repos(root: &Path) -> Result<HashMap<String, Repo>> {
    let mut results = HashMap::<String, Repo>::new();

    for entry in walkdir::WalkDir::new(root).max_depth(1) {
        let entry = entry.context("enumerating directory entry")?;
        let path = entry.path();
        if !path.is_dir() || path.eq(root) {
            continue;
        }

        match Repo::new(entry.path()) {
            Ok(repo) => {
                let name = String::from(
                    entry
                        .file_name()
                        .to_str()
                        .expect("Repo name contains non-UTF-8 characters"),
                );
                results.insert(name, repo);
            }
            Err(e) => {
                error!("Ignoring path {:?} ({:?})", entry.path(), e);
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use crate::testing::scratch_git_repo::ScratchGitRepo;
    use anyhow::Result;
    use env_logger::Env;
    use tempfile::tempdir;

    fn init_logging() {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    }

    #[test]
    fn test_find_repos() -> Result<()> {
        init_logging();
        let containing_dir = tempdir()?;
        let path = containing_dir.path();

        let repos = super::find_repos(&path)?;
        assert!(repos.is_empty());

        let repo_a = ScratchGitRepo::new_fixture(&path)?;
        let repo_a_name = repo_a.path().file_name().unwrap().to_str().unwrap();

        let repo_b = ScratchGitRepo::new_fixture(&path)?;
        let repo_b_name = repo_b.path().file_name().unwrap().to_str().unwrap();

        let repos = super::find_repos(&path)?;
        assert_eq!(repos.len(), 2);

        assert!(repos.contains_key(repo_a_name));
        assert!(repos.contains_key(repo_b_name));

        Ok(())
    }
}
