// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use focus_util::app::{App, ExitCode};
use focus_util::git_helper;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

const FOCUS_SYNC_REF: &str = "refs/focus/sync";
// This is correct for `source`
const PREFETCH_DEFAULT_REF: &str = "refs/prefetch/remotes/origin/master";

/// Entry point for pull operation
/// Run preflight checks: ref existence, prefetch and focus/sync refs are ahead of `HEAD`
/// Update refs/remotes/origin/* from refs/prefetch/*, and
/// Update current branch from focus/sync
#[tracing::instrument]
pub fn run(app: Arc<App>, repo_path: PathBuf) -> Result<ExitCode> {
    preflight_checks(app.clone(), repo_path.as_path())?;
    update_refs_from_prefetch(app.clone(), repo_path.as_path())?;
    update_current_branch(app, repo_path.as_path())?;

    Ok(ExitCode(0))
}

/// Validate existence of refs that `focus pull` operates on
fn validation_ref_existence(app: Arc<App>, repo_path: &Path) -> Result<()> {
    let focus_sync_ref = git_helper::parse_ref(app.clone(), repo_path, FOCUS_SYNC_REF);
    let prefetch_default_ref = git_helper::parse_ref(app, repo_path, PREFETCH_DEFAULT_REF);

    if focus_sync_ref.is_err() || focus_sync_ref.unwrap().is_empty() {
        bail!("Could not find focus sync ref or ref is empty");
    }
    if prefetch_default_ref.is_err() || prefetch_default_ref.unwrap().is_empty() {
        bail!("Could not find prefetch ref or ref is empty");
    }

    Ok(())
}

/// Validate that `prefetch` refs are ahead of current HEAD, and
/// validate that `focus/sync` ref is ahead of current HEAD
fn validate_merge_base(app: Arc<App>, repo_path: &Path) -> Result<()> {
    let default_prefetch_ref_sha =
        git_helper::parse_ref(app.clone(), repo_path, PREFETCH_DEFAULT_REF)
            .expect("Could not parse default prefetch ref");
    let focus_sync_ref_sha = git_helper::parse_ref(app.clone(), repo_path, FOCUS_SYNC_REF)
        .expect("Could not parse `refs/focus/sync`");
    let current_head = git_helper::get_current_revision(app.clone(), repo_path)?;

    // If prefetch refs and HEAD are equal then we can exit early
    if default_prefetch_ref_sha == current_head {
        bail!("HEAD is up to date with prefetch, nothing to do.")
    }

    let merge_base_prefetch_and_head = git_helper::get_merge_base(
        app.clone(),
        repo_path,
        &current_head,
        PREFETCH_DEFAULT_REF,
        None,
    )
    .context(
        "Could not get merge-base between current HEAD and 'refs/prefetch/remotes/origin/master'",
    )?;

    // If prefetch is behind current HEAD, then exit early
    if merge_base_prefetch_and_head == default_prefetch_ref_sha {
        bail!("Exiting: Prefetch is behind HEAD");
    }

    // Tests fail if I take out the redundant clone here
    #[allow(clippy::redundant_clone)]
    let merge_base_focus_sync_and_head =
        git_helper::get_merge_base(app.clone(), repo_path, &current_head, FOCUS_SYNC_REF, None)
            .context("Could not get merge-base between current HEAD and 'refs/focus/sync'")?;

    // If focus sync is behind current HEAD, then exit early
    if merge_base_focus_sync_and_head == focus_sync_ref_sha {
        bail!("Exiting: refs/focus/sync is behind HEAD");
    }

    Ok(())
}

fn preflight_checks(app: Arc<App>, repo_path: &Path) -> Result<()> {
    validation_ref_existence(app.clone(), repo_path)?;
    validate_merge_base(app, repo_path)?;

    Ok(())
}

fn update_refs_from_prefetch(app: Arc<App>, repo_path: &Path) -> Result<()> {
    info!("Fetching changes from `refs/prefetch/remotes/origin/*`");
    // TODO: don't assume remote names
    git_helper::fetch_refs(
        repo_path,
        ["+refs/prefetch/remotes/origin/*:refs/remotes/origin/*"].iter(),
        ".",
        app,
        None,
    )
    .context("Fetching from prefetch")?;

    Ok(())
}

/// Update the current branch from `refs/focus/sync`
fn update_current_branch(app: Arc<App>, repo_path: &Path) -> Result<()> {
    let current_branch = git_helper::get_current_branch(app.clone(), repo_path)
        .context("Could not get current branch")?;
    if current_branch.is_empty() {
        bail!("HEAD does not point to a branch");
    }
    info!("Updating current branch from `refs/focus/sync`");
    git_helper::pull(
        repo_path,
        [format!("refs/focus/sync:refs/{}", current_branch)].iter(),
        ".",
        app,
        None,
        None,
    )
    .context("Could not update current branch")?;

    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::pull::{validate_merge_base, validation_ref_existence};
    use anyhow::Result;
    use focus_testing::ScratchGitRepo;
    use focus_util::app::App;
    use focus_util::git_helper;
    use std::sync::Arc;

    #[test]
    fn test_preflight_check() -> Result<()> {
        let temp_sparse_dir = tempfile::tempdir()?;
        let scratch_repo = ScratchGitRepo::new_static_fixture(temp_sparse_dir.path())?;
        let repo_dir = scratch_repo.path();
        let app = Arc::new(App::new_for_testing()?);
        let head = git_helper::get_current_revision(app.clone(), repo_dir)?;

        // ref existence check should fail when the required refs don't exist
        assert!(validation_ref_existence(app.clone(), repo_dir).is_err());

        // Create `refs/focus/sync`
        let _ = git_helper::run_consuming_stdout(
            repo_dir,
            ["update-ref", "refs/focus/sync", &head],
            app.clone(),
        )?;

        // Create a default prefetch ref
        let _ = git_helper::run_consuming_stdout(
            repo_dir,
            ["update-ref", "refs/prefetch/remotes/origin/master", &head],
            app.clone(),
        )?;

        // All required refs exist, ref existence check should succeed
        assert!(validation_ref_existence(app.clone(), repo_dir).is_ok());

        // Create a default remote ref
        let _ = git_helper::run_consuming_stdout(
            repo_dir,
            ["update-ref", "refs/remotes/origin/master", &head],
            app.clone(),
        )?;

        // Since all refs point to the same commit, merge base validation should fail
        assert!(validate_merge_base(app.clone(), repo_dir).is_err());

        // Create a new branch and a new commit
        scratch_repo.create_and_switch_to_branch("test-branch")?;

        // Add a commit to our test branch, we will use this commit to move prefetch ahead
        scratch_repo.make_empty_commit("test commit", None)?;
        let new_commit = git_helper::get_current_revision(app.clone(), repo_dir)?;

        // Update prefetch default to new commit
        let _ = git_helper::run_consuming_stdout(
            repo_dir,
            [
                "update-ref",
                "refs/prefetch/remotes/origin/master",
                &new_commit,
            ],
            app.clone(),
        )?;

        // Switch back to `main` branch
        let _ = git_helper::run_consuming_stdout(repo_dir, ["checkout", "main"], app.clone())?;

        // Since focus/sync still points to `main` HEAD, merge base validation should still fail
        assert!(validate_merge_base(app.clone(), repo_dir).is_err());

        // Update focus/sync to new commit
        let _ = git_helper::run_consuming_stdout(
            repo_dir,
            ["update-ref", "refs/focus/sync", &new_commit],
            app.clone(),
        )?;

        assert!(validate_merge_base(app, repo_dir).is_ok());

        Ok(())
    }
}
