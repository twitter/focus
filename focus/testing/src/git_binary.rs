// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Clone, Debug)]
pub struct GitBinary {
    pub home_temp_dir: Option<Arc<TempDir>>,
    pub git_binary_path: PathBuf,
    pub git_exec_path: PathBuf,
}

impl PartialEq for GitBinary {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            home_temp_dir: _,
            git_binary_path,
            git_exec_path,
        } = self;
        let Self {
            home_temp_dir: _,
            git_binary_path: other_git_binary_path,
            git_exec_path: other_git_exec_path,
        } = other;

        git_binary_path == other_git_binary_path && git_exec_path == other_git_exec_path
    }
}

impl Eq for GitBinary {}

fn home_temp_dir() -> anyhow::Result<Option<Arc<TempDir>>> {
    if cfg!(test) {
        Ok(Some(Arc::new(tempfile::tempdir()?)))
    } else {
        Ok(None)
    }
}

impl GitBinary {
    pub fn from_env() -> anyhow::Result<Self> {
        let home_temp_dir = home_temp_dir()?;
        let git_binary_path =
            which::which("git").context("Resolving absolute path for git binary")?;
        let git_exec_path = git_exec_path(&git_binary_path)?;
        Ok(Self {
            home_temp_dir,
            git_binary_path,
            git_exec_path,
        })
    }

    pub fn from_binary_path(git_binary_path: PathBuf) -> anyhow::Result<Self> {
        let home_temp_dir = home_temp_dir()?;
        let git_exec_path = git_exec_path(&git_binary_path)?;
        Ok(Self {
            home_temp_dir,
            git_binary_path,
            git_exec_path,
        })
    }

    pub fn command(&self) -> Command {
        if cfg!(test) {
            let mut command = Command::new(&self.git_binary_path);
            command.env_clear();
            command.env("HOME", &self.git_exec_path);
            command.env("GIT_EXEC_PATH", &self.git_exec_path);
            command.env("GIT_AUTHOR_NAME", "Focus Testing");
            command.env("GIT_AUTHOR_EMAIL", "focus@example.com");
            command.env("GIT_COMMITTER_NAME", "Focus Testing");
            command.env("GIT_COMMITTER_EMAIL", "focus@example.com");
            command
        } else {
            Command::new(&self.git_binary_path)
        }
    }
}

const NL: u8 = b'\n';

fn git_exec_path(git_binary_path: &Path) -> anyhow::Result<PathBuf> {
    let mut output = Command::new(git_binary_path)
        .arg("--exec-path")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git --exec-path failed to run");
    }

    let stdout = &mut output.stdout;
    if *stdout.last().unwrap() == NL {
        stdout.pop();
    }

    let out = OsString::from_vec(output.stdout);
    Ok(PathBuf::from(out).canonicalize()?)
}
