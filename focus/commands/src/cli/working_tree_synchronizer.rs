use anyhow::{bail, Context, Error, Result};
use std::path::{Path, PathBuf};

use git2::{Repository, RepositoryState};

use crate::{
    sandbox::Sandbox,
    sandbox_command::{SandboxCommand, SandboxCommandOutput},
};

#[derive(thiserror::Error, Debug)]
pub(crate) enum SyncError {
    #[error("Repo path is not a directory")]
    RepoPath,

    #[error("Working tree cannot be dirty")]
    DirtyWorkingTree,
}
pub struct WorkingTreeSynchronizer<'this> {
    path: PathBuf,
    sandbox: &'this Sandbox,
}

impl<'this> WorkingTreeSynchronizer<'this> {
    pub(crate) fn new(path: &Path, sandbox: &'this Sandbox) -> Result<Self> {
        if !path.is_dir() {
            return Err(Error::new(SyncError::RepoPath));
        }

        Ok(Self {
            path: path.to_path_buf(),
            sandbox,
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

    pub(crate) fn read_head(&self) -> Result<Vec<u8>> {
        use crate::temporary_working_directory::TemporaryWorkingDirectory;
        use std::process;

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

    pub fn read_branch(&self) -> Result<String> {
        use std::process;

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

    pub(crate) fn get_merge_base(&self, reference: &str) -> Result<Vec<u8>> {
        use crate::temporary_working_directory::TemporaryWorkingDirectory;
        use std::process;

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

    pub fn checkout_orphaned(&self, commit_identifier: &str) -> Result<()> {
        let (mut cmd, scmd) = SandboxCommand::new("git", &self.sandbox)?;

        scmd.ensure_success_or_log(
            cmd.arg("checkout")
                .arg("--orphaned")
                .arg(commit_identifier)
                .current_dir(&self.path),
            SandboxCommandOutput::Ignore,
            "checking out orphaned commit",
        )
        .context("checking out orphaned commit")?;

        Ok(())
    }

    // pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
    //     let (mut cmd, scmd) = SandboxCommand::new("git", &self.sandbox)?;

    //     scmd.ensure_success_or_log(
    //         cmd.arg("remote")
    //             .arg("add")
    //             .arg(name)
    //             .arg(url)
    //             .current_dir(&self.path),
    //         SandboxCommandOutput::Ignore,
    //         "adding a remote",
    //     )
    //     .context("adding a remote")?;

    //     Ok(())
    // }

    // pub fn remove_remote(&self, name: &str) -> Result<()> {
    //     let (mut cmd, scmd) = SandboxCommand::new("git", &self.sandbox)?;

    //     scmd.ensure_success_or_log(
    //         cmd.arg("remote")
    //             .arg("remove")
    //             .arg(name)
    //             .current_dir(&self.path),
    //         SandboxCommandOutput::Ignore,
    //         "removing a remote",
    //     )
    //     .context("removing a remote")?;

    //     Ok(())
    // }

    pub fn push_to_remote(&self, remote: &str, name: &str) -> Result<()> {
        let (mut cmd, scmd) = SandboxCommand::new("git", &self.sandbox)?;

        scmd.ensure_success_or_log(
            cmd.arg("push")
                .arg(remote)
                .arg(name)
                .current_dir(&self.path),
            SandboxCommandOutput::Ignore,
            "push to remote",
        )
        .context("push to remote")?;

        Ok(())
    }

    // pub fn push_temp(&self, commit_identifier: &str) -> Result<()> {
    //     let (mut cmd, scmd) = SandboxCommand::new("git", &self.sandbox)?;

    //     scmd.ensure_success_or_log(
    //         cmd.arg("checkout")
    //             .arg("--orphaned")
    //             .arg(commit_identifier)
    //             .current_dir(&self.path),
    //         SandboxCommandOutput::Ignore,
    //         "checking out orphaned commit",
    //     )
    //     .context("checking out orphaned commit")?;

    //     Ok(())
    // }

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
        let sandbox = Sandbox::new(false).unwrap();
        let containing_dir = tempdir().unwrap();
        let original = ScratchGitRepo::new_fixture(&containing_dir.path()).unwrap();
        let cloned = original.make_clone().unwrap();

        let original_sync = WorkingTreeSynchronizer::new(&original.path(), &sandbox).unwrap();
        let clone_sync = WorkingTreeSynchronizer::new(&cloned.path(), &sandbox).unwrap();
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
