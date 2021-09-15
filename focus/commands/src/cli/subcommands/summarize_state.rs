use anyhow::{bail, Context, Result};

use std::path::{Path, PathBuf};

use crate::git_helper;
use crate::{sandbox::Sandbox, sparse_repos::Spec};

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

    let revspec = format!("{}..HEAD", &sync_state.as_str());
    let output = git_helper::run_git_command_consuming_stdout(
        repo,
        vec!["diff", "--name-only", revspec.as_str()],
        sandbox,
    )?;
    let changed_paths: Vec<&str> = output.lines().collect::<_>();
    let mut build_involved_changed_paths = Vec::<PathBuf>::new();
    for line in &changed_paths {
        log::info!("line:{}", line);
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
    if find_uncommitted_changes(sandbox, repo).context("detecting uncommitted changes")? {
        eprintln!("Uncommitted changes affect the build graph, you must run `focus sync` to synchronized the sparse checkout!");
        std::process::exit(1);
    } else if find_committed_changes(sandbox, repo).context("detecting committed changes")? {
        eprintln!("Committed changes affect the build graph, you must run `focus sync` to synchronized the sparse checkout!");
        std::process::exit(1);
    }

    Ok(())
}
