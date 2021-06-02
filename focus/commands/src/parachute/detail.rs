use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
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
    fn new(path: &Path) -> Result<Self, AppError> {
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
pub(crate) fn find_repos(root: &Path) -> Result<HashMap<String, Repo>, AppError> {
    let mut results = HashMap::<String, Repo>::new();

    for entry in walkdir::WalkDir::new(root).max_depth(1) {
        match entry {
            Ok(entry) =>
                {
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
            },
            Err(e) => {
                return Err(AppError::Io(
                    e.into_io_error().expect("Converting error failed"),
                ))
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use crate::testing::git_test_helper::GitTestHelper;
    use env_logger::Env;
    use internals::error::AppError;
    use tempfile::tempdir;

    fn init_logging() {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    }

    #[test]
    fn test_find_repos() -> Result<(), AppError> {
        init_logging();
        let containing_dir = tempdir()?;

        let repos = super::find_repos(containing_dir.path())?;
        assert!(repos.is_empty());

        let repo_a = GitTestHelper::fixture_repo(containing_dir.path())?;
        let repo_a_name = repo_a.file_name().unwrap().to_str().unwrap();

        let repo_b = GitTestHelper::fixture_repo(containing_dir.path())?;
        let repo_b_name = repo_b.file_name().unwrap().to_str().unwrap();

        let repos = super::find_repos(containing_dir.path())?;
        assert_eq!(repos.len(), 2);

        assert!(repos.contains_key(repo_a_name));
        assert!(repos.contains_key(repo_b_name));

        Ok(())
    }
}
