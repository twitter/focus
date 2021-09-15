use anyhow::{bail, Context, Result};

use std::path::{Path, PathBuf};

use std::sync::mpsc;

use crate::git_helper;
use crate::sandbox::Sandbox;

fn build_graph_involved_filename_predicate(name: &Path) -> bool {
    if let Some(extension) = name.extension() {
        if extension.eq("bzl") {
            return true;
        }
    }
    if let Some(file_name) = name.file_name() {
        return file_name.eq("BUILD") || file_name.eq("WORKSPACE");
    }

    false
}

fn find_committed_changes(sandbox: &Sandbox, repo: &PathBuf) -> Result<bool> {
    let sync_state = git_helper::read_config(repo.as_path(), "twitter.focus.sync-point", sandbox)
        .context("reading sync state")?;

    let revspec = format!("{}..HEAD", &sync_state.trim());
    let output = git_helper::run_git_command_consuming_stdout(
        repo,
        vec!["diff", "--name-only", revspec.as_str()],
        sandbox,
    )?;
    let changed_paths: Vec<&str> = output.lines().collect::<_>();
    let mut build_involved_changed_paths = Vec::<PathBuf>::new();
    for line in &changed_paths {
        let parsed = PathBuf::from(line);
        if build_graph_involved_filename_predicate(parsed.as_path()) {
            log::info!("Committed {}", parsed.display());
            build_involved_changed_paths.push(parsed);
        }
    }
    Ok(!&changed_paths.is_empty())
}

fn find_uncommitted_changes(sandbox: &Sandbox, repo: &PathBuf) -> Result<bool> {
    let output = git_helper::run_git_command_consuming_stdout(
        repo,
        vec!["status", "--porcelain", "--no-renames"],
        sandbox,
    )?;
    let all_changes: Vec<&str> = output.lines().collect::<_>();
    let mut build_involved_changed_paths = Vec::<PathBuf>::new();
    for line in &all_changes {
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
        if build_graph_involved_filename_predicate(parsed.as_path()) {
            log::info!("Uncommitted {}", parsed.display());
            build_involved_changed_paths.push(parsed);
        }
    }

    Ok(!&build_involved_changed_paths.is_empty())
}

pub fn run(sandbox: &Sandbox, repo: &PathBuf) -> Result<()> {
    let (uncommitted_tx, uncommitted_rx) = mpsc::channel();
    let uncommited_finder_thread = {
        let cloned_repo = repo.clone();
        let cloned_sandbox = sandbox.clone();

        std::thread::spawn(move || {
            uncommitted_tx
                .send(
                    find_uncommitted_changes(&cloned_sandbox, &cloned_repo)
                        .expect("error detecting uncommitted changes"),
                )
                .expect("send failed");
        })
    };

    let (committed_tx, committed_rx) = mpsc::channel();
    let committed_finder_thread = {
        let cloned_repo = repo.clone();
        let cloned_sandbox = sandbox.clone();

        std::thread::spawn(move || {
            committed_tx
                .send(
                    find_committed_changes(&cloned_sandbox, &cloned_repo)
                        .expect("error detecting committed changes"),
                )
                .expect("send failed");
        })
    };

    let has_uncommitted_changes = uncommitted_rx
        .recv()
        .expect("could not receive whether there were uncommitted changes");
    let has_committed_changes = committed_rx
        .recv()
        .expect("could not receive whether there were committed changes");

    uncommited_finder_thread
        .join()
        .expect("thread crashed detecting uncommitted changes");
    committed_finder_thread
        .join()
        .expect("thread crashed detecting uncommitted changes");

    if has_committed_changes && has_uncommitted_changes {
        eprintln!("Committed and uncommitted changes affect the build graph, you must run `focus sync` to synchronized the sparse checkout!");
        std::process::exit(1);
    } else if has_committed_changes {
        eprintln!("Committed changes affect the build graph, you must run `focus sync` to synchronized the sparse checkout!");
        std::process::exit(1);
    } else if has_uncommitted_changes {
        eprintln!("Uncommitted changes affect the build graph, you must run `focus sync` to synchronized the sparse checkout!");
        std::process::exit(1);
    }
    log::info!("No changes to files affecting the build graph were detected");
    Ok(())
}
