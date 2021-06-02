use std::collections::{HashSet, HashMap};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::Result;
use env_logger::{self, Env};
use git2::{ObjectType, TreeEntry, TreeWalkMode, TreeWalkResult, Repository};
use log::{debug, error, info};
use sha2::{Digest, Sha224, Sha256};
use sha2::digest::DynDigest;
use structopt::StructOpt;
use internals::error::AppError;
use std::ffi::{OsStr, OsString};
use focus_formats::parachute::*;

lazy_static! {
    static ref BARE_REPO_EXTENSION: OsString = OsString::from("git");
}

pub(crate) struct Repo {
    repo: Repository,
}

impl Repo {
    fn new(path: &Path) -> Result<Self, AppError> {
        Repository::open(path).map(|repo| {
            Self{
                repo
            }
        }).map_err(|err| err.into())
    }
}

pub fn server(repo: &Path, data: &Path) -> Result<(), AppError> {
    todo!("impl")
}

pub(crate) fn find_repos(root: &Path) -> Result<HashMap<String, Repo>, AppError> {
    let mut results = HashMap::<String, Repo>::new();

    for entry in walkdir::WalkDir::new(root) {
        match entry {
            Ok(entry) => {
                match Repo::new(entry.path()) {
                    Ok(repo) => {
                        let name = String::from(entry.file_name().to_str().expect("Repo name contains non-UTF-8 characters"));
                        results.insert(name, repo);
                    },
                    Err(e) => {
                        error!("Ignoring repository {:?} ({:?})", entry.path(), e);
                    }
                }
            },
            Err(e) => {
                return Err(AppError::Io(e.into_io_error().expect("Converting error failed")))
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use env_logger::Env;
    use git2::{ObjectType, Oid, Tree, TreeEntry, TreeWalkMode, TreeWalkResult};
    use log::info;
    use tempfile::{tempdir, TempDir};

    use internals::error::AppError;
    use internals::fixtures::scm::testing::TempRepo;

    use crate::testing::git_test_helper::GitTestHelper;

    fn init_logging() {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    }

    struct DirNode {}

    #[test]
    fn test_find_repos() -> Result<(), AppError> {
        init_logging();
        let containing_dir = tempdir()?;
        let repo_a = GitTestHelper::fixture_repo(containing_dir.path())?;
        let repo_a_name = repo_a.to_str().unwrap();
        let repo_b = GitTestHelper::fixture_repo(containing_dir.path())?;
        let repo_b_name = repo_b.to_str().unwrap();

        let repos= super::find_repos(containing_dir.path())?;
        assert_eq!(repos.len(), 2);
        assert!(repos.contains_key(repo_a_name));
        assert!(repos.contains_key(repo_b_name));


        Ok(())
    }
}
