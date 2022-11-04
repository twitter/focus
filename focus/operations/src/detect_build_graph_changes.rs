// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use tracing::{info, warn};

use std::path::{Path, PathBuf};

use std::sync::{mpsc, Arc};

use focus_internals::model::repo::Repo;
use focus_util::{
    app::{App, ExitCode},
    git_helper, paths,
};

fn find_committed_changes(app: Arc<App>, repo_path: &Path) -> Result<Vec<PathBuf>> {
    let repo = Repo::open(repo_path, app.clone())?;
    let working_tree = {
        if let Some(t) = repo.working_tree() {
            t
        } else {
            bail!("No working tree");
        }
    };

    let sync_state_oid = {
        if let Some(sync_point) = working_tree
            .read_sparse_sync_point_ref()
            .context("reading sync state")?
        {
            sync_point
        } else {
            bail!("No sync state!");
        }
    };

    let revspec = format!("{}..HEAD", &sync_state_oid);
    let output =
        git_helper::run_consuming_stdout(repo_path, ["diff", "--name-only", &revspec], app)?;
    let mut build_involved_changed_paths = Vec::<PathBuf>::new();
    for line in output.lines() {
        let parsed = PathBuf::from(line);
        if paths::is_relevant_to_build_graph(parsed.as_path()) {
            info!(path = ?parsed, "Committed path");
            build_involved_changed_paths.push(parsed);
        }
    }
    Ok(build_involved_changed_paths)
}

fn find_uncommitted_changes(app: Arc<App>, repo: &Path) -> Result<Vec<PathBuf>> {
    let output =
        git_helper::run_consuming_stdout(repo, ["status", "--porcelain", "--no-renames"], app)?;
    let mut build_involved_changed_paths = Vec::<PathBuf>::new();
    for line in output.lines() {
        let mut tokens = line.split_ascii_whitespace().take(2);
        let status = tokens.next();
        if status.is_none() {
            bail!("missing first token parsing line {}", &line);
        }
        let path = tokens.next();
        if path.is_none() {
            bail!("missing second token parsing line {}", &line);
        }
        let parsed = PathBuf::from(path.unwrap());
        if paths::is_relevant_to_build_graph(parsed.as_path()) {
            info!(path = ?parsed, "Uncommitted file");
            build_involved_changed_paths.push(parsed);
        }
    }

    Ok(build_involved_changed_paths)
}

fn is_ignored_subcommand(subcommand: &str) -> bool {
    // Twitter specific behavior: never stop `bazel lint` (as if that's a thing).
    // TODO: Turn this into config later.
    subcommand.eq_ignore_ascii_case("lint")
}

#[cfg(target_os = "macos")]
#[allow(unused_variables)]
fn notify(repo: &Repo, repo_name: &str, message: &str, _persistent: bool) -> Result<()> {
    warn!(repo = repo_name, message);

    #[cfg(not(test))]
    {
        use focus_internals::model::configuration::NotificationCategory;
        if repo
            .config()
            .notification
            .is_allowed(NotificationCategory::BuildGraphState)
        {
            let subtitle = format!("\u{1F4C1} {} \u{1F3AF} Focused Repo", repo_name);
            let _ = notify_rust::Notification::new()
                .appname("focus")
                .subtitle(&subtitle)
                .body(message)
                .show();
        }
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn notify(repo: &Repo, _subtitle: &str, message: &str, _persistent: bool) -> Result<()> {
    warn!(repo_path = ?repo.git_dir(), message);
    Ok(())
}

pub fn run(repo_path: &Path, advisory: bool, args: Vec<String>, app: Arc<App>) -> Result<ExitCode> {
    if let Some(subcommand) = args.get(0) {
        if is_ignored_subcommand(subcommand) {
            return Ok(ExitCode(0));
        }
    }

    // TODO: Consider removing uncommitted change detection since we can't perform operations in repos without a clean working tree anyway.
    let (uncommitted_tx, uncommitted_rx) = mpsc::channel();
    let uncommited_finder_thread = {
        let cloned_repo = repo_path.to_path_buf();
        let cloned_sandbox = app.clone();

        std::thread::spawn(move || {
            uncommitted_tx
                .send(
                    find_uncommitted_changes(cloned_sandbox.clone(), &cloned_repo)
                        .expect("error detecting uncommitted changes"),
                )
                .expect("send failed");
        })
    };

    let (committed_tx, committed_rx) = mpsc::channel();
    let committed_finder_thread = {
        let cloned_repo = repo_path.to_path_buf();
        let cloned_sandbox = app.clone();

        std::thread::spawn(move || {
            committed_tx
                .send(
                    find_committed_changes(cloned_sandbox, &cloned_repo)
                        .expect("error detecting committed changes"),
                )
                .expect("send failed");
        })
    };

    let uncommitted_changes = uncommitted_rx
        .recv()
        .expect("could not receive whether there were uncommitted changes");
    let committed_changes = committed_rx
        .recv()
        .expect("could not receive whether there were committed changes");

    uncommited_finder_thread
        .join()
        .expect("thread crashed detecting uncommitted changes");
    committed_finder_thread
        .join()
        .expect("thread crashed detecting uncommitted changes");

    let failing_exit_code = if advisory { ExitCode(0) } else { ExitCode(1) }; // If we are running in advisory mode, just report the error and exit 0.

    // Treat the repo's file name as the title of the repo. It should be absolute in most cases since `main` sends us the result of calling `git rev-parse --show-toplevel`, which canonicalizes paths. For tests, etc, we treat the name as "unknown" otherwise.
    let repo_name = if repo_path.is_absolute() {
        repo_path.file_name().unwrap().to_str().unwrap_or("Unknown")
    } else {
        "Unknown"
    };

    let repo = Repo::open(repo_path, app)?;
    if !committed_changes.is_empty() && !uncommitted_changes.is_empty() {
        notify(&repo, repo_name, "Committed and uncommitted changes affect the build graph, please commit changes and run `focus sync` to update the sparse checkout!", true)?;
        Ok(failing_exit_code)
    } else if !committed_changes.is_empty() {
        notify(&repo, repo_name, "Committed changes affect the build graph, please run `focus sync` to update the sparse checkout!", true)?;
        Ok(failing_exit_code)
    } else if !uncommitted_changes.is_empty() {
        notify(&repo, repo_name, "Uncommitted changes affect the build graph, please commit changes and run `focus sync` to update the sparse checkout!", true)?;
        Ok(failing_exit_code)
    } else {
        // Don't notify if there are no changes, it's annoying
        info!(
            repo = repo_name,
            "No changes to files affecting the build graph were detected"
        );
        Ok(ExitCode(0))
    }
}
