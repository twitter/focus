use std::path::{Path, PathBuf};
use git2::{Repository, RepositoryState};
use tokio::io::AsyncReadExt;
use std::ffi::OsString;
use std::error::Error;

#[derive(Error, Debug)]
pub(crate) enum SyncError {
    #[error("Repo path is not a directory")]
    RepoPath,

    #[error("Repo could not be opened")]
    OpeningRepo(git2::Error),

    #[error("Working tree cannot be dirty")]
    DirtyWorkingTree,

    #[error("Subprocess failed")]
    SubprocessFailed,
}

pub(crate) struct Synchronizer {
    path: PathBuf,
}

impl Synchronizer {     pub(crate) fn new(path: &Path) -> Result<Self, SyncError> {
        if !path.is_dir() {
            return Err(SyncError::RepoPath);
        }

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn get_repo_hash_bits(&self) -> Result<u8, Box<dyn Error>> {
        let repo = Repository::open(&self.path).expect("opening repo failed");
        let config = repo.config().expect("reading config failed");
        if let Ok(s) = config.get_str("extensions.objectFormat") {
            if s.eq_ignore_ascii_case("sha256") {
                return Ok(256)
            }
        }

        Ok(128)
    }

    pub(crate) fn get_merge_base(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        use tokio::process;
        use std::io::prelude::*;
        use std::io::BufReader;

        let mut cmd = process::Command::new("git")
            .arg("-C")
            .arg(self.path.as_os_str())
            .arg("show-branch")
            .arg("--merge-base")
            .spawn()
            .expect("git show-branch failed");

        let stdout = cmd.stdout.take()
            .expect("missing stdout handle");

        let mut reader = BufReader::new(stdout).lines();

        tokio::spawn(async move {
            child.wait().await
                .expect("git show-branch failed");
        });

        while let Some(line) = reader.next_line().await? {
            // We only care about the first line
            return Ok(line.bytes())
        }

        Err(Box::new(SyncError::SubprocessFailed))
    }

    pub(crate) fn create_snapshot(&self) -> Result<(), SyncError> {
        let repo = Repository::open(self.path.as_path()).expect("Failed to open repo");

        match repo.state() {
            RepositoryState::Clean => {},
            _ => {
                return Err(SyncError::DirtyWorkingTree.into());
            }
        }

        repo.index().

        Ok(())
    }
}

// To test:
// Symlinks
// Deletions
// Index vs working tree