#![allow(dead_code)]
use std::{
    collections::HashSet,
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{PathBuf, Path},
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use once_cell::{sync::Lazy, unsync::OnceCell};
use tracing::warn;
use walkdir::{WalkDir, DirEntry};

static _MARKER_VERSION: &str = "METAHOOK_v202203240001";

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

const MHOOKS_RELPATH: &str = "hooks_multi";

const GIT_HOOKS_STDIN: &[&str] = &["pre-push", "pre-receive", "post-receive", "post-rewrite"];

static VALID_NAMES: Lazy<HashSet<&'static str>> =
    Lazy::new(|| HashSet::from_iter(GIT_HOOKS.iter().map(|name| *name)));

#[derive(Debug, Clone)]
struct MultiHookRunner {
    git_dir_cell: OnceCell<PathBuf>
}

fn find_git_dir() -> Result<PathBuf> {
    let data = Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        .context("failed to run git rev-parse --git-dir")?
        .stdout;

    let pb = PathBuf::from(OsStr::from_bytes(&data));
    if pb.is_dir() {
        let c = pb
            .canonicalize()
            .with_context(|| format!("failed to canonicalize path {:?}", pb))?;
        Ok(c)
    } else {
        bail!("could not determine git dir from returned path: {:?}", pb)
    }
}

fn is_valid_hook_name(name: &str) -> bool {
    GIT_HOOKS.iter().any(|n| *n == name)
}

fn is_hook_stdin(name: &str) -> bool {
    GIT_HOOKS_STDIN.iter().any(|n| *n == name)
}


impl MultiHookRunner {
    fn new() -> Self {
        Self {
            git_dir_cell: OnceCell::new(),
        }
    }

    fn git_dir(&self) -> Result<&Path> {
        self.git_dir_cell.get_or_try_init(|| find_git_dir())
            .map(|p| p as &Path )
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

    fn mhooks_path<P: AsRef<Path>>(&self, p: P) -> Result<PathBuf> {
        Ok(self.git_dir()?.join(MHOOKS_RELPATH).join(p.as_ref()))
    }

    fn hooks_d_path<S: AsRef<str>>(&self, p: S) -> Result<PathBuf> {
        self.mhooks_path(format!("{}.d", p.as_ref()))
    }

    fn is_runcom(p: &DirEntry) -> Result<bool> {
        use std::os::unix::fs::PermissionsExt;

        if let Some(fname) = p.file_name().to_str() {
            if fname.starts_with(".") {
                return Ok(false)
            }

            let meta = p.metadata()?;
            if !meta.is_file() {
                warn!(?p, "path is not a regular file");
                return Ok(false)
            }
            let perms = p.metadata()


            

        } else {
            warn!(?p, "path contained invalid UTF-8 characters");
            return Ok(false)
        }


        Ok(true)
    }

    fn get_runcoms(&self, hook_name: &str) -> Result<Vec<PathBuf>> {
        let mulithooks_dir = self.hooks_d_path(hook_name)?;
        let mut result: Vec<PathBuf> = Vec::new();

        for entry in WalkDir::new(mulithooks_dir)
            .min_depth(1)
            .max_depth(1)
            .sort_by_file_name() {

            let entry = entry?;
            if Self::is_runcom(&entry)? {
                result.push(entry.into_path())
            }
        }

        Ok(result)
    }

    fn run(&self) -> Result<()> {
        let hook_name = Self::get_hook_name()?;
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
}



// fn get_runcoms(hook_name: &str) -> Result<Vec<String>> {

// }





fn main() -> Result<()> {
    let mhr = MultiHookRunner::new();
    mhr.run()
}
