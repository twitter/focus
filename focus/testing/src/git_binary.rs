// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use std::collections::HashMap;
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
    pub env: HashMap<OsString, OsString>,
}

impl PartialEq for GitBinary {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            home_temp_dir: _,
            git_binary_path,
            git_exec_path,
            env,
        } = self;
        let Self {
            home_temp_dir: _,
            git_binary_path: other_git_binary_path,
            git_exec_path: other_git_exec_path,
            env: other_env,
        } = other;

        git_binary_path == other_git_binary_path
            && git_exec_path == other_git_exec_path
            && env == other_env
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
        let env = std::env::vars_os().collect();
        Ok(Self {
            home_temp_dir,
            git_binary_path,
            git_exec_path,
            env,
        })
    }

    pub fn from_binary_path(git_binary_path: PathBuf) -> anyhow::Result<Self> {
        let home_temp_dir = home_temp_dir()?;
        let git_exec_path = git_exec_path(&git_binary_path)?;
        let env = std::env::vars_os().collect();
        Ok(Self {
            home_temp_dir,
            git_binary_path,
            git_exec_path,
            env,
        })
    }

    pub fn for_testing() -> anyhow::Result<Self> {
        let home_temp_dir = home_temp_dir()?;
        let git_binary_path =
            which::which("git").context("Resolving absolute path for git binary")?;
        let git_exec_path = git_exec_path(&git_binary_path)?;
        let env = [
            ("HOME", git_exec_path.clone().into_os_string()),
            ("GIT_EXEC_PATH", git_exec_path.clone().into_os_string()),
            ("GIT_AUTHOR_NAME", OsString::from("Focus Testing")),
            ("GIT_AUTHOR_EMAIL", OsString::from("focus@example.com")),
            ("GIT_COMMITTER_NAME", OsString::from("Focus Testing")),
            ("GIT_COMMITTER_EMAIL", OsString::from("focus@example.com")),
        ];
        let env = env
            .into_iter()
            .map(|(k, v)| (OsString::from(k), v))
            .collect();
        Ok(Self {
            home_temp_dir,
            git_binary_path,
            git_exec_path,
            env,
        })
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.git_binary_path);
        command.env_clear();
        command.envs(self.env.iter());
        command
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
