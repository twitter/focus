use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{TempDir, tempdir};

use internals::error::AppError;
use uuid::Uuid;

pub struct GitTestHelper {}

impl GitTestHelper {
    pub fn fixture_repo(containing_dir: &Path) -> Result<PathBuf, AppError> {
        let mut repo_path = containing_dir.to_path_buf();
        std::env::set_current_dir(containing_dir);
        let name = format!("repo-{}", Uuid::new_v4().to_string());
        Command::new("git")
            .args(vec!["init", &name, "-b", "main"])
            .spawn()?
            .wait()
            .expect("init failed");
        repo_path.push(Path::new(&name));
        std::env::set_current_dir(repo_path.as_path());
        let mut test_file = repo_path.to_path_buf();
        test_file.push("d_0_0");
        std::fs::create_dir(test_file.as_path());
        test_file.push("f_1.txt");
        std::fs::write(test_file.as_path(), &"This is test file 1"[..]).unwrap();
        test_file.pop();
        test_file.push("d_0_1");
        std::fs::create_dir(test_file.as_path());
        test_file.push("f_2.txt");
        std::fs::write(test_file.as_path(), &"This is test file 2"[..]).unwrap();
        test_file.pop();
        test_file.pop();
        test_file.pop();
        test_file.push("d_1_0");
        std::fs::create_dir(test_file.as_path());
        test_file.push("f_3.txt");
        std::fs::write(test_file.as_path(), &"This is test file 3"[..]).unwrap();

        Command::new("git")
            .args(vec!["add", "--", "."])
            .spawn()?
            .wait()
            .expect("add failed");

        Command::new("git")
            .args(vec!["commit", "-a", "-m", "Test commit"])
            .spawn()?
            .wait()
            .expect("commit failed");

        Ok(repo_path)
    }
}

#[cfg(test)]
mod tests {
    use internals::error::AppError;
    use tempfile::tempdir;
    use crate::testing::git_test_helper::GitTestHelper;
    use git2::Repository;

    #[test]
    fn test_git_test_helper() -> Result<(), AppError> {
        let containing_dir = tempdir()?;
        if let Ok(repo_dir) = GitTestHelper::fixture_repo(containing_dir.path()) {
            let repo = Repository::open(repo_dir);
            assert!(repo.is_ok());
            let repo = repo.unwrap();
            assert!(repo.head().is_ok());
        }
        Ok(())
    }
}
