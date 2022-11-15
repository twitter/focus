// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    cell::Cell,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result};

use assert_cmd::prelude::OutputAssertExt;
use tempfile::TempDir;

use tracing::{info, warn};

use focus_testing::ScratchGitRepo;
use focus_util::app::App;

use focus_internals::{model::repo::Repo, tracker::Tracker};

use crate::{
    clone::CloneArgs,
    sync::{SyncMode, SyncRequest},
};

pub use focus_testing::GitBinary;

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
    pub tracker: Tracker,
    pub app: Arc<App>,
    pub preserve: bool,
    pub sync_mode: Cell<SyncMode>,
}

impl RepoPairFixture {
    pub fn new() -> Result<Self> {
        let app = Arc::new(App::new_for_testing()?);
        let dir = TempDir::new()?;
        let dense_repo_path = dir.path().join("dense");
        let branch = String::from("main");
        let dense_repo = ScratchGitRepo::new_copied_fixture(
            app.git_binary().clone(),
            Path::new("bazel_java_example"),
            &dense_repo_path,
            &branch,
        )?;
        let sparse_repo_path = dir.path().join("sparse");
        let projects_and_targets: Vec<String> = vec![];
        let tracker = Tracker::for_testing()?;
        tracker.ensure_directories_exist()?;
        let fixture = Self {
            dir,
            dense_repo_path,
            sparse_repo_path,
            dense_repo,
            branch,
            projects_and_targets,
            app,
            tracker,
            preserve: false,
            sync_mode: Cell::new(SyncMode::Incremental),
        };
        Ok(fixture)
    }

    pub fn with_sync_mode(mode: SyncMode) -> Result<Self> {
        let instance = Self::new()?;
        instance.sync_mode.replace(mode);
        Ok(instance)
    }

    fn ensure_sync_mode(&self) -> Result<()> {
        self.sparse_repo()?
            .set_bazel_oneshot_resolution(self.sync_mode.get() == SyncMode::OneShot)?;
        Ok(())
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
        let clone_args = CloneArgs {
            origin: Some(crate::clone::Origin::Local(self.dense_repo_path.clone())),
            branch: self.branch.clone(),
            projects_and_targets: self.projects_and_targets.clone(),
            copy_branches: true,
            days_of_history: 90,
            do_post_clone_fetch: false,
            sync_mode: self.sync_mode.get(),
        };

        crate::clone::run(
            self.sparse_repo_path.clone(),
            clone_args,
            None,
            &self.tracker,
            self.app.clone(),
        )?;

        self.ensure_sync_mode()
    }

    #[allow(dead_code)]
    pub fn perform_sync(&self) -> Result<bool> {
        self.ensure_sync_mode()?;
        crate::sync::run(
            &SyncRequest::new(&self.sparse_repo_path, self.sync_mode.get()),
            self.app.clone(),
        )
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
        self.app
            .git_binary()
            .command()
            .arg("fetch")
            .arg(remote_name)
            .current_dir(path)
            .assert()
            .success();
        let fetch_head_path = path.join(".git").join("FETCH_HEAD");
        Self::parse_fetch_head(fetch_head_path)
    }

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

        self.app
            .git_binary()
            .command()
            .arg("pull")
            .arg(remote_name)
            .arg(branch)
            .current_dir(path)
            .assert()
            .success();

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
            if let Some(outlining_tree) = sparse_repo.outliner() {
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

#[cfg(feature = "twttr")]
pub fn configure_ci_for_dense_repo(fixture: &RepoPairFixture) -> Result<()> {
    let binary = fixture.app.git_binary();

    binary
        .command()
        .arg("config")
        .arg("--add")
        .arg("--local")
        .arg("ci.alt.remote")
        .arg("https://git.twitter.biz/focus-test-repo-ci")
        .current_dir(&fixture.dense_repo_path)
        .assert()
        .try_success()?;

    binary
        .command()
        .arg("config")
        .arg("--add")
        .arg("--local")
        .arg("--bool")
        .arg("ci.alt.enabled")
        .arg("true")
        .current_dir(&fixture.dense_repo_path)
        .assert()
        .try_success()?;

    Ok(())
}

#[cfg(feature = "twttr")]
#[cfg(test)]
mod twttr_test {
    use super::RepoPairFixture;
    use anyhow::Result;
    use focus_testing::init_logging;

    #[test]
    fn test_ci_config_is_set_in_dense_repo() -> Result<()> {
        init_logging();
        let fixture = RepoPairFixture::new()?;
        super::configure_ci_for_dense_repo(&fixture)?;

        let repo = fixture.dense_repo.repo()?;

        assert_eq!(
            repo.config()?.get_string("ci.alt.remote")?,
            "https://git.twitter.biz/focus-test-repo-ci"
        );
        assert!(repo.config()?.get_bool("ci.alt.enabled")?);

        Ok(())
    }
}
