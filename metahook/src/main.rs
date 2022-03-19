#![allow(dead_code)]
use std::{
    collections::HashSet,
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;

const GIT_HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "post-receive",
    "post-update",
    "pre-auto-gc",
    "post-rewrite",
    "post-gc",
    "pre-checkout-branch",
    "post-journal-fetch",
];

const GIT_HOOKS_STDIN: &[&str] = &["pre-push", "pre-receive", "post-receive", "post-rewrite"];

static VALID_NAMES: Lazy<HashSet<&'static str>> =
    Lazy::new(|| HashSet::from_iter(GIT_HOOKS.iter().map(|name| *name)));

fn git_dir() -> Result<PathBuf> {
    let data = Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        .context("failed to run git rev-parse --git-dir")?
        .stdout;

    let pb = PathBuf::from(OsStr::from_bytes(&data));
    if pb.is_dir() {
        Ok(pb
            .canonicalize()
            .with_context(|| format!("failed to canonicalize path {:?}", pb))?)
    } else {
        bail!("could not determine git dir from returned path: {:?}", pb)
    }
}

fn get_hook_name() -> Result<String> {
    let cur = std::env::current_exe()?;
    if let Some(name) = cur.file_name() {
        if let Some(name) = name.to_str() {
            Ok(name.to_owned())
        } else {
            bail!(
                "current_exe name contains invalid UTF-8 sequences: {:?}",
                cur
            );
        }
    } else {
        bail!(
            "current_exe returned a path without a file_name component: {:?}",
            cur
        );
    }
}

fn is_valid_hook_name(name: &str) -> bool {
    GIT_HOOKS.iter().any(|n| *n == name)
}

fn is_hook_stdin(name: &str) -> bool {
    GIT_HOOKS_STDIN.iter().any(|n| *n == name)
}

fn main() -> Result<()> {
    let hook_name = get_hook_name()?;
    if !is_valid_hook_name(&hook_name) {
        bail!("invoked as {}: not a valid hook name", hook_name)
    }
    let hook_input = if is_hook_stdin(&hook_name) {
        Stdio::inherit()
    } else {
        Stdio::null()
    };

    Ok(())
}
