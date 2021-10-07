use anyhow::{bail, Context, Result};
use git2::Repository;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::util::temporary_working_directory::TemporaryWorkingDirectory;

#[allow(dead_code)]
pub struct ScratchGitRepo {
    path: PathBuf,
}

impl ScratchGitRepo {
    // Create a new fixture repo with a unique random name in the given directory
    #[allow(unused)]
    pub fn new_fixture(containing_dir: &Path) -> Result<Self> {
        Ok(Self {
            path: Self::create_fixture_repo(containing_dir)?,
        })
    }

    // Create a new repo by cloning another repo from the local filesystem
    pub fn new_local_clone(local_origin: &Path) -> Result<Self> {
        let uuid = uuid::Uuid::new_v4();
        let mut target_path = local_origin.to_owned();
        target_path.set_extension(format!("clone_{}", &uuid));
        Self::create_local_cloned_repo(local_origin, target_path.as_path())?;

        Ok(Self {
            path: target_path.to_owned(),
        })
    }

    #[allow(dead_code)]
    pub fn make_clone(&self) -> Result<Self> {
        Self::new_local_clone(self.path())
    }

    #[allow(dead_code)]
    pub(crate) fn create_fixture_repo(containing_dir: &Path) -> Result<PathBuf> {
        let name = format!("repo_{}", Uuid::new_v4().to_string());
        Command::new("git")
            .arg("init")
            .arg(&name)
            .current_dir(containing_dir)
            .status()
            .expect("git init failed");
        let repo_path = containing_dir.join(&name);

        Command::new("git")
            .arg("switch")
            .arg("-c")
            .arg("main")
            .current_dir(&repo_path)
            .status()
            .expect("git switch failed");

        let mut test_file = repo_path.to_path_buf();
        test_file.push("d_0_0");
        std::fs::create_dir(test_file.as_path()).unwrap();
        test_file.push("f_1.txt");
        std::fs::write(test_file.as_path(), &"This is test file 1"[..]).unwrap();
        test_file.pop();
        test_file.push("d_0_1");
        std::fs::create_dir(test_file.as_path())?;
        test_file.push("f_2.txt");
        std::fs::write(test_file.as_path(), &"This is test file 2"[..]).unwrap();
        test_file.pop();
        test_file.pop();
        test_file.pop();
        test_file.push("d_1_0");
        std::fs::create_dir(test_file.as_path())?;
        test_file.push("f_3.txt");
        std::fs::write(test_file.as_path(), &"This is test file 3"[..]).unwrap();

        Command::new("git")
            .arg("add")
            .arg("--")
            .arg(".")
            .current_dir(&repo_path)
            .status()
            .expect("add failed");

        Command::new("git")
            .arg("commit")
            .arg("-a")
            .arg("-m")
            .arg("Test commit")
            .current_dir(&repo_path)
            .status()
            .expect("commit failed");

        Ok(repo_path)
    }

    pub(crate) fn create_local_cloned_repo(origin: &Path, destination: &Path) -> Result<()> {
        if !origin.is_absolute() {
            bail!("origin path must be absolute");
        }
        let mut qualified_origin = OsString::from("file://");
        qualified_origin.push(origin.as_os_str());
        Command::new("git")
            .args(vec![
                &OsString::from("clone"),
                &qualified_origin,
                &OsString::from(destination.as_os_str()),
            ])
            .spawn()?
            .wait()
            .expect("clone failed");
        Ok(())
    }

    #[allow(dead_code)]
    pub fn commit(&self, filename: &Path, content: &[u8], message: &str) -> Result<git2::Oid> {
        let _wd = TemporaryWorkingDirectory::new(self.path.as_path());

        // Write the file
        std::fs::write(filename, content).context("writing content")?;

        // Run `git add`
        if !Command::new("git")
            .arg("add")
            .arg("--")
            .arg(filename.as_os_str())
            .spawn()
            .context("running `git add`")?
            .wait()
            .context("`git add` failed")?
            .success()
        {
            bail!("`git add` exited abnormally");
        }

        // Run `git commit`
        if !Command::new("git")
            .arg("commit")
            .arg("-a")
            .arg("-m")
            .arg(message)
            .spawn()
            .context("running `git commit`")?
            .wait()
            .context("`git commit` failed")?
            .success()
        {
            bail!("`git commit` exited abnormally");
        }

        // Read the commit hash
        let repo = self.repo()?;
        let id = repo
            .head()
            .context("reading HEAD reference")?
            .peel_to_commit()
            .context("finding commit")?
            .id();
        Ok(id)
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[allow(dead_code)]
    pub fn repo(&self) -> Result<Repository> {
        Ok(Repository::open(&self.path).context("opening repository")?)
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::scratch_git_repo::ScratchGitRepo;
    use anyhow::Result;
    use git2::Repository;
    use tempfile::tempdir;

    #[test]
    fn test_git_test_helper() -> Result<()> {
        let containing_dir = tempdir()?;
        if let Ok(repo) = ScratchGitRepo::new_fixture(containing_dir.path()) {
            let repo = Repository::open(repo.path());
            assert!(repo.is_ok());
        }
        Ok(())
    }
}
