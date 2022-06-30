// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitBinary {
    pub git_binary_path: PathBuf,
    pub git_exec_path: PathBuf,
}

impl GitBinary {
    pub fn from_env() -> anyhow::Result<Self> {
        let git_binary_path =
            which::which("git").context("Resolving absolute path for git binary")?;
        let git_exec_path = git_exec_path(&git_binary_path)?;
        Ok(Self {
            git_binary_path,
            git_exec_path,
        })
    }

    pub fn from_binary_path(git_binary_path: PathBuf) -> anyhow::Result<Self> {
        let git_exec_path = git_exec_path(&git_binary_path)?;
        Ok(Self {
            git_binary_path,
            git_exec_path,
        })
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.git_binary_path);
        command.env_clear();
        command.env("GIT_EXEC_PATH", &self.git_exec_path);
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
