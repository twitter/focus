use anyhow::{bail, Context, Error, Result};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use git2::{Repository, RepositoryState};

use crate::{
    app::App, ui::ProgressReporter, util::temporary_working_directory::TemporaryWorkingDirectory,
};

#[derive(thiserror::Error, Debug)]
pub(crate) enum SyncError {
    #[error("Repo path is not a directory")]
    RepoPath,

    #[error("Working tree cannot be dirty")]
    DirtyWorkingTree,
}
pub struct WorkingTreeSynchronizer {
    path: PathBuf,
    app: Arc<App>,
}

impl WorkingTreeSynchronizer {
    pub(crate) fn new(path: &Path, app: Arc<App>) -> Result<Self> {
        if !path.is_dir() {
            return Err(Error::new(SyncError::RepoPath));
        }

        Ok(Self {
            path: path.to_path_buf(),
            app,
        })
    }

    // Determine the number of bits used in the repo's hash function
    #[allow(dead_code)]
    pub(crate) fn get_repo_hash_bits(&self) -> Result<usize> {
        let repo = Repository::open(&self.path).expect("opening repo failed");
        let config = repo.config().expect("reading config failed");
        if let Ok(s) = config.get_str("extensions.objectFormat") {
            if s.eq_ignore_ascii_case("sha256") {
                return Ok(256);
            }
        }

        Ok(128)
    }

    pub fn is_working_tree_clean(&self) -> Result<bool> {
        use std::process;
        let _progress = ProgressReporter::new(
            self.app.clone(),
            format!(
                "Determining the state of the working tree in repo {}",
                self.path.display()
            ),
        );
        let output = process::Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .current_dir(&self.path)
            .output()
            .context("running `git status`")?;
        if !output.status.success() {
            bail!("git status failed");
        }
        let output_str = String::from_utf8(output.stdout).context("parsing output as UTF-8")?;
        Ok(output_str.trim().is_empty())
    }

    #[allow(dead_code)]
    pub(crate) fn read_head(&self) -> Result<Vec<u8>> {
        use std::process;
        let _progress = ProgressReporter::new(
            self.app.clone(),
            format!(
                "Determining the current revision in repo {}",
                self.path.display()
            ),
        );
        let _wd = TemporaryWorkingDirectory::new(self.path.as_path());
        let output = process::Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .context("running `git rev-parse`")?;
        if !output.status.success() {
            bail!("git rev-parse failed");
        }
        let output_str = String::from_utf8(output.stdout).context("parsing output as UTF-8")?;

        Ok(Vec::from(
            output_str
                .split_whitespace()
                .next()
                .expect("expected output")
                .as_bytes(),
        ))
    }

    #[allow(dead_code)]
    pub fn read_branch(&self) -> Result<String> {
        use std::process;
        let _progress = ProgressReporter::new(
            self.app.clone(),
            format!(
                "Determining the current branch in repo {}",
                self.path.display()
            ),
        );

        let output = process::Command::new("git")
            .arg("branch")
            .arg("--show-current")
            .current_dir(&self.path)
            .output()
            .context("running `git branch`")?;
        if !output.status.success() {}
        let output_str = String::from_utf8(output.stdout).context("parsing output as UTF-8")?;

        Ok(output_str
            .split_whitespace()
            .next()
            .expect("expected output")
            .to_owned())
    }

    #[allow(dead_code)]
    pub(crate) fn get_merge_base(&self, reference: &str) -> Result<Vec<u8>> {
        use std::process;
        let _progress = ProgressReporter::new(
            self.app.clone(),
            format!("Determining the merge base in repo {}", self.path.display()),
        );

        let _wd = TemporaryWorkingDirectory::new(self.path.as_path());
        let output = process::Command::new("git")
            .arg("show-branch")
            .arg("--merge-base")
            .arg(reference)
            .output()
            .context("running `git show-branch --merge-base`")?;
        if !output.status.success() {}
        let output_str = String::from_utf8(output.stdout).context("parsing output as UTF-8")?;

        Ok(Vec::from(
            output_str
                .split_whitespace()
                .next()
                .expect("expected output")
                .as_bytes(),
        ))
    }

    #[allow(dead_code)]
    pub(crate) fn create_snapshot(&self) -> Result<()> {
        let repo = Repository::open(self.path.as_path()).expect("Failed to open repo");

        match repo.state() {
            RepositoryState::Clean => {}
            _ => {
                return Err(SyncError::DirtyWorkingTree.into());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_git_repo::ScratchGitRepo;
    use tempfile::tempdir;

    #[test]
    fn test_get_merge_base() {
        let app = Arc::from(App::new(false, false).unwrap());
        let containing_dir = tempdir().unwrap();
        let original = ScratchGitRepo::new_fixture(&containing_dir.path()).unwrap();
        let cloned = original.make_clone().unwrap();
        let cloned_app = app.clone();
        let original_sync = WorkingTreeSynchronizer::new(&original.path(), cloned_app).unwrap();
        let cloned_app = app.clone();
        let clone_sync = WorkingTreeSynchronizer::new(&cloned.path(), cloned_app).unwrap();
        cloned
            .commit(
                Path::new("quotes.txt"),
                "The rain in Spain falls mainly on the plain.".as_bytes(),
                "Add a quote file",
            )
            .unwrap();

        assert_eq!(
            clone_sync.get_merge_base("origin/main").unwrap(),
            original_sync.read_head().unwrap()
        );
    }
}

// To test:
// Symlinks
// Deletions
// Index vs working tree
