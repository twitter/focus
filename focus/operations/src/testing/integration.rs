use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result};

use tempfile::TempDir;

use tracing::{info, warn};

use focus_testing::ScratchGitRepo;
use focus_util::app::App;

use focus_internals::model::repo::Repo;

#[allow(dead_code)]
pub enum RepoDisposition {
    Dense,
    Sparse,
}

#[allow(dead_code)]
pub struct RepoPairFixture {
    pub dir: TempDir,
    pub dense_repo_path: PathBuf,
    pub sparse_repo_path: PathBuf,
    pub dense_repo: ScratchGitRepo,
    pub branch: String,
    pub projects_and_targets: Vec<String>,
    pub app: Arc<App>,
    pub preserve: bool,
}

impl RepoPairFixture {
    #[allow(dead_code)]
    pub fn new() -> Result<Self> {
        let dir = TempDir::new()?;
        let dense_repo_path = dir.path().join("dense");
        let branch = String::from("main");
        let dense_repo = ScratchGitRepo::new_copied_fixture(
            Path::new("bazel_java_example"),
            &dense_repo_path,
            &branch,
        )?;
        let sparse_repo_path = dir.path().join("sparse");
        let projects_and_targets: Vec<String> = vec![];
        let app = Arc::new(App::new_for_testing()?);

        Ok(Self {
            dir,
            dense_repo_path,
            sparse_repo_path,
            dense_repo,
            branch,
            projects_and_targets,
            app,
            preserve: false,
        })
    }

    fn preserve_dirs(&mut self) -> Result<()> {
        let path = self.dir.path();
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("failed to get parent path"))?;
        let preserved = parent.join(format!("focus-test-{}", chrono::Utc::now().to_rfc3339()));
        match std::fs::rename(path, &preserved) {
            Ok(_) => info!("preserved test dirs to {:?}", &preserved),
            Err(_) => warn!("failed to preserve test directories"),
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn perform_clone(&self) -> Result<()> {
        crate::clone::run(
            crate::clone::Origin::Local(self.dense_repo_path.clone()),
            self.sparse_repo_path.clone(),
            self.branch.clone(),
            self.projects_and_targets.clone(),
            true,
            90,
            self.app.clone(),
        )
    }

    #[allow(dead_code)]
    pub fn perform_sync(&self) -> Result<bool> {
        crate::sync::run(&self.sparse_repo_path, false, self.app.clone())
            .map(|result| result.checked_out)
    }

    #[allow(dead_code)]
    pub fn parse_fetch_head(path: impl AsRef<Path>) -> Result<Vec<git2::Oid>> {
        let mut results: Vec<git2::Oid> = Vec::new();

        let path = path.as_ref();

        let reader = BufReader::new(
            File::open(path).with_context(|| format!("Opening {}", path.display()))?,
        );

        for line in reader.lines().flatten() {
            let mut tokens = line.split_ascii_whitespace();
            if let Some(token) = tokens.next() {
                results.push(git2::Oid::from_str(token).context("Parsing OID")?);
            }
        }

        Ok(results)
    }

    #[allow(dead_code)]
    pub fn perform_fetch(
        &self,
        repo: RepoDisposition,
        remote_name: &str,
    ) -> Result<Vec<git2::Oid>> {
        let path = match repo {
            RepoDisposition::Dense => &self.dense_repo_path,
            RepoDisposition::Sparse => &self.sparse_repo_path,
        };
        Command::new("git")
            .arg("fetch")
            .arg(remote_name)
            .current_dir(&path)
            .status()
            .expect("git pull failed");
        let fetch_head_path = path.join(".git").join("FETCH_HEAD");
        Self::parse_fetch_head(fetch_head_path)
    }

    #[allow(dead_code)]
    pub fn perform_pull(
        &self,
        repo: RepoDisposition,
        remote_name: &str,
        branch: &str,
    ) -> Result<()> {
        let path = match repo {
            RepoDisposition::Dense => &self.dense_repo_path,
            RepoDisposition::Sparse => &self.sparse_repo_path,
        };

        Command::new("git")
            .arg("pull")
            .arg(remote_name)
            .arg(branch)
            .current_dir(&path)
            .status()
            .expect("git pull failed");

        Ok(())
    }

    #[allow(dead_code)]
    /// Open a Repo instance modeling the sparse repo
    pub fn sparse_repo(&self) -> Result<Repo> {
        Repo::open(&self.sparse_repo_path, self.app.clone())
    }

    /// Stop Bazel if it is running in both repos
    fn stop_bazel(&self) {
        let mut commands = vec![(
            Command::new("bazel")
                .arg("shutdown")
                .current_dir(&self.dense_repo_path)
                .spawn(),
            self.dense_repo_path.to_owned(),
        )];
        if let Ok(sparse_repo) = self.sparse_repo() {
            if let Some(working_tree) = sparse_repo.working_tree() {
                commands.push((
                    Command::new("bazel")
                        .arg("shutdown")
                        .current_dir(working_tree.work_dir())
                        .spawn(),
                    working_tree.work_dir().to_owned(),
                ));
            }
            if let Some(outlining_tree) = sparse_repo.outlining_tree() {
                commands.push((
                    Command::new("bazel")
                        .arg("shutdown")
                        .current_dir(outlining_tree.underlying().work_dir())
                        .spawn(),
                    outlining_tree.underlying().work_dir().to_owned(),
                ));
            }
        }

        for spawn in commands {
            match spawn {
                (Ok(mut child), path) => {
                    // We can't really do anything anyway.
                    match child.wait() {
                        Ok(status) => {
                            if status.code() == Some(0) {
                                info!("Bazel shutdown in {} shutdown succeeded", path.display());
                            } else {
                                warn!("Bazel shutdown in {} failed", path.display());
                            }
                        }
                        Err(e) => warn!(
                            "Failed to wait for Bazel shutdown in {}: {}",
                            path.display(),
                            e
                        ),
                    }
                }
                (Err(e), path) => warn!(
                    "Failed to spawn Bazel shutdown in {}: {}",
                    path.display(),
                    e
                ),
            }
        }
    }
}

impl Drop for RepoPairFixture {
    fn drop(&mut self) {
        self.stop_bazel();
        if self.preserve {
            // Ignore return value.
            self.preserve_dirs().ok();
        }
    }
}
