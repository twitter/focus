use anyhow::{bail, Context, Result};
use git2::Repository;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use super::fixture_dir;

pub struct ScratchGitRepo {
    path: PathBuf,
}

impl ScratchGitRepo {
    // Create a new fixture repo with a unique random name in the given directory
    pub fn new_static_fixture(containing_dir: &Path) -> Result<Self> {
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

    // Create a new copied fixture
    pub fn new_copied_fixture(
        fixture_name: &Path,
        destination_path: &Path,
        branch: &str,
    ) -> Result<Self> {
        if destination_path.exists() {
            bail!("Destination path {} exists", destination_path.display());
        }

        let fixture_path = fixture_dir()?.join("repos").join(fixture_name);
        assert!(fixture_path.is_absolute());

        // Copy to destination dir
        Command::new("cp")
            .arg("-r")
            .arg(fixture_path)
            .arg(destination_path)
            .status()
            .expect("copy failed");

        // Initialize the destination path as a Git repo
        Command::new("git")
            .arg("init")
            .current_dir(destination_path)
            .status()
            .expect("init failed");

        // Create the named branch
        Command::new("git")
            .arg("checkout")
            .arg("--force")
            .arg("-b")
            .arg(branch)
            .current_dir(destination_path)
            .status()
            .expect("checkout branch failed");

        // Add everything and commit it
        Command::new("git")
            .arg("add")
            .arg("--")
            .arg(".")
            .current_dir(&destination_path)
            .status()
            .expect("add failed");

        Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg("Initial import")
            .current_dir(&destination_path)
            .status()
            .expect("commit failed");
        Ok(Self {
            path: destination_path.to_owned(),
        })
    }

    pub fn make_clone(&self) -> Result<Self> {
        Self::new_local_clone(self.path())
    }

    pub(crate) fn create_fixture_repo(containing_dir: &Path) -> Result<PathBuf> {
        let name = format!("repo_{}", Uuid::new_v4());
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

        let mut test_file = repo_path.clone();
        test_file.push("d_0_0");
        std::fs::create_dir(test_file.as_path()).unwrap();
        test_file.push("f_1.txt");
        std::fs::write(test_file.as_path(), "This is test file 1").unwrap();
        test_file.pop();
        test_file.push("d_0_1");
        std::fs::create_dir(test_file.as_path())?;
        test_file.push("f_2.txt");
        std::fs::write(test_file.as_path(), "This is test file 2").unwrap();
        test_file.pop();
        test_file.pop();
        test_file.pop();
        test_file.push("d_1_0");
        std::fs::create_dir(test_file.as_path())?;
        test_file.push("f_3.txt");
        std::fs::write(test_file.as_path(), "This is test file 3").unwrap();

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
            .args(&[
                OsStr::new("clone"),
                &qualified_origin,
                destination.as_os_str(),
            ])
            .spawn()?
            .wait()
            .expect("clone failed");
        Ok(())
    }

    pub fn write_file(
        &self,
        relative_filename: impl AsRef<Path>,
        content: impl AsRef<[u8]>,
    ) -> Result<()> {
        let relative_filename = relative_filename.as_ref();
        let absolute_filename = self.path.join(relative_filename);
        if let Some(parent_dir) = absolute_filename.parent() {
            std::fs::create_dir_all(parent_dir).context("creating intermediate directories")?;
        }
        std::fs::write(&absolute_filename, content).context("writing content")?;
        Ok(())
    }

    pub fn add_file(&self, relative_filename: impl AsRef<Path>) -> Result<()> {
        if !Command::new("git")
            .arg("add")
            .arg("--")
            .arg(relative_filename.as_ref())
            .current_dir(&self.path)
            .spawn()
            .context("running `git add`")?
            .wait()
            .context("`git add` failed")?
            .success()
        {
            bail!("`git add` exited abnormally");
        }
        Ok(())
    }

    pub fn commit_all(&self, message: impl AsRef<str>) -> Result<git2::Oid> {
        // Run `git commit`
        if !Command::new("git")
            .arg("commit")
            .arg("-a")
            .arg("-m")
            .arg(message.as_ref())
            .current_dir(&self.path)
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

    pub fn write_and_commit_file(
        &self,
        relative_filename: impl AsRef<Path>,
        content: impl AsRef<[u8]>,
        message: impl AsRef<str>,
    ) -> Result<git2::Oid> {
        let relative_filename = relative_filename.as_ref();
        self.write_file(relative_filename, content)?;
        self.add_file(relative_filename)?;
        self.commit_all(message)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn repo(&self) -> Result<Repository> {
        Repository::open(&self.path).context("opening repository")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use git2::Repository;
    use tempfile::tempdir;

    #[test]
    fn test_git_test_helper() -> Result<()> {
        let containing_dir = tempdir()?;
        if let Ok(repo) = ScratchGitRepo::new_static_fixture(containing_dir.path()) {
            let repo = Repository::open(repo.path());
            assert!(repo.is_ok());
        }
        Ok(())
    }
}
