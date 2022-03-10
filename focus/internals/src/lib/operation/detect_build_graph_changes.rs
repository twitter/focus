use anyhow::{bail, Context, Result};
use tracing::info;

use std::path::{Path, PathBuf};

use std::sync::{mpsc, Arc};

use crate::app::{App, ExitCode};
use crate::model::repo::Repo;
use crate::util::{git_helper, paths};

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
            .read_sync_point_ref()
            .context("reading sync state")?
        {
            sync_point
        } else {
            bail!("No sync state!");
        }
    };

    let revspec = format!("{}..HEAD", &sync_state_oid);
    let description = format!(
        "Finding committed changes since the last sync point ({})",
        &revspec
    );
    let output = git_helper::run_consuming_stdout(
        description,
        repo_path,
        &["diff", "--name-only", &revspec],
        app,
    )?;
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
    let output = git_helper::run_consuming_stdout(
        "Finding uncommitted changes",
        repo,
        &["status", "--porcelain", "--no-renames"],
        app,
    )?;
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
            info!(path = ?parsed, "Uncommitted path");
            build_involved_changed_paths.push(parsed);
        }
    }

    Ok(build_involved_changed_paths)
}

pub fn run(repo: &Path, args: Vec<String>, app: Arc<App>) -> Result<ExitCode> {
    if let Some(verb) = args.get(0) {
        if verb.eq_ignore_ascii_case("lint") {
            // Twitter specific behavior: Do not stop `bazel lint` running under `arc lint` as happens frequently. Act as if we didn't run.
            return Ok(ExitCode(0));
        }
    }

    // TODO: Consider removing uncommitted change detection since we can't perform operations in repos without a clean working tree anyway.
    let (uncommitted_tx, uncommitted_rx) = mpsc::channel();
    let uncommited_finder_thread = {
        let cloned_repo = repo.to_path_buf();
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
        let cloned_repo = repo.to_path_buf();
        let cloned_sandbox = app;

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

    if !committed_changes.is_empty() && !uncommitted_changes.is_empty() {
        eprintln!("Committed and uncommitted changes affect the build graph, please run `focus sync` to update the sparse checkout!");
        Ok(ExitCode(1))
    } else if !committed_changes.is_empty() {
        eprintln!("Committed changes affect the build graph, please run `focus sync` to update the sparse checkout!");
        Ok(ExitCode(1))
    } else if !uncommitted_changes.is_empty() {
        eprintln!("Uncommitted changes affect the build graph, please run `focus sync` to update the sparse checkout!");
        Ok(ExitCode(1))
    } else {
        eprintln!("No changes to files affecting the build graph were detected");
        Ok(ExitCode(0))
    }
}
