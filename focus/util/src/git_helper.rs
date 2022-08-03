// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{ffi::OsStr, path::PathBuf, process::Stdio, str::FromStr, sync::Arc};

use anyhow::{anyhow, bail, Context, Result};
use git2;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;
use std::process::Command;
use tracing::{error, warn};

use crate::{
    app::App,
    sandbox_command::{SandboxCommand, SandboxCommandOutput},
    time::GitIdentTime,
};

use super::time::{FocusTime, GitTime};

pub use focus_testing::GitBinary;

pub fn git_dir(path: &Path) -> Result<PathBuf> {
    Ok(git2::Repository::open(path)?.path().to_path_buf())
}

pub fn git_command(app: Arc<App>) -> Result<(Command, SandboxCommand)> {
    let git_binary = app.git_binary();
    let mut cmd = git_binary.command();
    let scmd = SandboxCommand::with_command(&mut cmd, app)?;
    Ok((cmd, scmd))
}

pub fn remote_add<P: AsRef<Path>>(
    repo_path: P,
    name: &str,
    url: &OsStr,
    app: Arc<App>,
) -> Result<()> {
    let (mut cmd, scmd) = git_command(app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path)
            .arg("remote")
            .arg("add")
            .arg(name)
            .arg(url),
        SandboxCommandOutput::Stderr,
    )
    .map(|_| ())
}

pub fn fetch_refs<P: AsRef<Path>>(
    repo_path: P,
    refspecs: impl Iterator<Item = impl AsRef<OsStr>>,
    remote: &str,
    app: Arc<App>,
    depth: Option<u64>,
) -> Result<()> {
    let (mut cmd, scmd) = git_command(app)?;
    cmd.current_dir(repo_path).arg("fetch").arg("--force");
    if let Some(d) = depth {
        cmd.arg(format!("--depth={}", d));
    }
    cmd.arg(remote);
    for s in refspecs {
        cmd.arg(s.as_ref());
    }
    scmd.ensure_success_or_log(&mut cmd, SandboxCommandOutput::Stderr)
        .map(|_| ())
}

pub fn fetch_all_tags<P: AsRef<Path>>(
    repo_path: P,
    remote: &str,
    app: Arc<App>,
    depth: Option<u64>,
) -> Result<String> {
    let mut args = vec![
        String::from("fetch"),
        remote.to_owned(),
        String::from("--prune"),
        String::from("--prune-tags"),
        String::from("--tags"),
        String::from("--force"),
        String::from("-k"),
    ];
    if let Some(d) = depth {
        args.push(format!("--depth={}", d));
    }
    run_consuming_stdout(repo_path.as_ref(), args, app)
}

pub fn ls_remote(remote: &str, app: Arc<App>) -> Result<String> {
    run_consuming_stdout(
        std::env::current_dir().unwrap(),
        vec!["ls-remote", "--tags", remote],
        app,
    )
}

pub fn push_refs<P: AsRef<Path>>(
    repo_path: P,
    refspecs: impl IntoIterator<Item = String>,
    remote: &str,
    app: Arc<App>,
) -> Result<()> {
    let mut args = vec![String::from("push"), remote.to_owned()];
    args.extend(refspecs);
    let (mut cmd, scmd) = git_command(app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).args(args),
        SandboxCommandOutput::Stderr,
    )
    .map(|_| ())
}

pub fn write_config<P: AsRef<Path>>(
    repo_path: P,
    key: &str,
    val: &str,
    app: Arc<App>,
) -> Result<()> {
    let (mut cmd, scmd) = git_command(app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).arg("config").arg(key).arg(val),
        SandboxCommandOutput::Stderr,
    )
    .map(|_| ())
}

pub fn read_config<P: AsRef<Path>>(
    repo_path: P,
    key: &str,
    app: Arc<App>,
) -> Result<Option<String>> {
    if let Ok(result) = run_consuming_stdout(repo_path, &["config", key], app) {
        return Ok(Some(result));
    }

    Ok(None)
}

pub fn unset_config<P: AsRef<Path>>(repo_path: P, key: &str, app: Arc<App>) -> Result<()> {
    let (mut cmd, _scmd) = git_command(app)?;
    cmd.arg("config")
        .arg("--unset")
        .arg(key)
        .current_dir(repo_path)
        .status()
        .with_context(|| format!("Running `git config --unset {}` failed", key))?;
    Ok(())
}

pub fn run_consuming_stdout<P, I, O>(repo: P, args: I, app: Arc<App>) -> Result<String>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = O>,
    O: AsRef<OsStr>,
{
    let (mut cmd, scmd) = git_command(app)?;
    if let Err(e) = cmd.current_dir(repo).args(args).status() {
        scmd.log(SandboxCommandOutput::Stderr, "git command")?;
        bail!("git command failed: {}", e);
    }
    let mut stdout_contents = String::new();
    scmd.read_to_string(SandboxCommandOutput::Stdout, &mut stdout_contents)?;
    Ok(stdout_contents.trim().to_owned())
}

pub fn find_top_level(app: Arc<App>, path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    if let Ok(path) = std::fs::canonicalize(path) {
        Ok(PathBuf::from(
            run_consuming_stdout(path, &["rev-parse", "--show-toplevel"], app)
                .context("Finding the repo's top level failed")?,
        ))
    } else {
        bail!(
            "Could not canonicalize repository path '{}'",
            &path.display()
        );
    }
}

pub fn get_current_revision(app: Arc<App>, repo: &Path) -> Result<String> {
    run_consuming_stdout(repo, &["rev-parse", "HEAD"], app)
}

pub fn get_current_branch(app: Arc<App>, repo: &Path) -> Result<String> {
    run_consuming_stdout(repo, &["branch", "--show-current"], app)
}

// Switches to a branch in a given repository, switching back to the previous branch afterwards
pub struct BranchSwitch {
    app: Arc<App>,
    repo: PathBuf,
    refname: String,
    alternate: Option<PathBuf>,
    switch_back: Option<String>,
}

impl BranchSwitch {
    pub fn permanent(
        app: Arc<App>,
        repo: PathBuf,
        refname: String,
        alternate: Option<PathBuf>,
    ) -> Result<Self> {
        let instance = Self {
            app,
            repo,
            refname,
            alternate,
            switch_back: None,
        };

        instance
            .switch(&instance.refname, true)
            .context("Switching branches failed")?;

        Ok(instance)
    }

    pub fn temporary(
        app: Arc<App>,
        repo: &Path,
        refname: String,
        alternate: Option<PathBuf>,
    ) -> Result<Self> {
        let current_branch = {
            let hint = get_current_branch(app.clone(), repo)?.trim().to_owned();
            if hint.is_empty() {
                error!(
                    "Couldn't determine the current branch in {}, using the default 'master'.",
                    repo.display()
                );

                String::from("master")
            } else {
                hint
            }
        };

        let instance = Self {
            app,
            repo: repo.to_path_buf(),
            refname,
            alternate,
            switch_back: Some(current_branch),
        };

        instance
            .switch(&instance.refname, true)
            .context("Switching branches failed")?;

        Ok(instance)
    }

    fn switch(&self, branch_name: &str, detach: bool) -> Result<()> {
        let (mut cmd, scmd) = git_command(self.app.clone())?;
        let cmd = cmd.arg("switch").current_dir(&self.repo);
        if detach {
            cmd.arg("--detach");
        }
        cmd.arg(&branch_name);

        if let Some(alternate_path) = &self.alternate {
            cmd.env(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                alternate_path.as_os_str(),
            );
        }
        scmd.ensure_success_or_log(cmd, SandboxCommandOutput::Stderr)?;

        Ok(())
    }
}

impl Drop for BranchSwitch {
    fn drop(&mut self) {
        if let Some(refname_to_switch_back_to) = &self.switch_back {
            self.switch(refname_to_switch_back_to, false)
                .expect("Switching back to the original branch failed");
        }
    }
}

pub fn get_head_commit(repo: &git2::Repository) -> Result<git2::Commit> {
    let head_reference = repo.head().context("resolving HEAD reference")?;
    let head_commit = head_reference
        .peel_to_commit()
        .context("resolving HEAD commit")?;
    Ok(head_commit)
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
/// Represents a git "ident", which is a signature and timestamp.
pub struct Ident {
    pub name: String,
    pub email: String,
    pub timestamp: FocusTime,
}

impl Ident {
    pub fn now(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            timestamp: FocusTime::now(),
        }
    }

    pub fn to_signature(&self) -> Result<git2::Signature<'static>> {
        let Self {
            name,
            email,
            timestamp,
        } = self;
        let git_time = GitTime::from(timestamp.clone());
        git2::Signature::new(name, email, &git_time.into_inner())
            .context("failed to create signature")
    }
}

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} <{}> {}",
            self.name,
            self.email,
            GitIdentTime::from(&self.timestamp),
        )
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

static VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^git version ([0-9]+)\.([0-9]+)\.([0-9]+)").unwrap());

impl GitVersion {
    pub fn current(git_binary: &GitBinary) -> Result<GitVersion> {
        let mut cmd = git_binary.command();
        let out = cmd.arg("version").stderr(Stdio::inherit()).output()?;

        let s = String::from_utf8(out.stdout)?;
        Self::from_str(&s)
    }
}

impl FromStr for GitVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match VERSION_RE.captures(s.as_ref()) {
            Some(caps) => {
                let major: u32 = caps[1].parse()?;
                let minor: u32 = caps[2].parse()?;
                let patch: u32 = caps[3].parse()?;

                Ok(GitVersion {
                    major,
                    minor,
                    patch,
                })
            }

            None => Err(anyhow::anyhow!(
                "could not parse version from string: {:?}",
                s
            )),
        }
    }
}

pub trait ConfigExt {
    fn multivar_values<S: AsRef<str>>(&self, name: S, regexp: Option<S>) -> Result<Vec<String>>;

    fn is_config_key_set<S: AsRef<str>>(&self, key: S) -> Result<bool>;
    fn set_str_if_not_set<S: AsRef<str>>(&mut self, key: S, value: S) -> Result<()>;
    fn get_bool_with_default<S: AsRef<str>>(&mut self, key: S, default: bool) -> Result<bool>;
    fn get_i64_with_default<S: AsRef<str>>(&mut self, key: S, default: i64) -> Result<i64>;
    fn dump_config(&self, glob: Option<&str>) -> Result<Vec<(String, String)>>;
}

impl ConfigExt for git2::Config {
    #[allow(dead_code)]
    fn multivar_values<S: AsRef<str>>(&self, name: S, regexp: Option<S>) -> Result<Vec<String>> {
        let configs = match regexp {
            Some(s) => self.multivar(name.as_ref(), Some(s.as_ref())),
            None => self.multivar(name.as_ref(), None),
        }?;

        let mut values: Vec<String> = Vec::new();

        for config_entry_r in configs.into_iter() {
            values.push(config_entry_r?.value().unwrap().to_owned());
        }

        Ok(values)
    }

    /// The git2 implementation of get_bool does not provide clear semantics around the
    /// key's existence. In the case where the key does not exist, this method returns
    /// the default.
    fn get_bool_with_default<S: AsRef<str>>(&mut self, key: S, default: bool) -> Result<bool> {
        if !self.is_config_key_set(key.as_ref())? {
            Ok(default)
        } else {
            match self.get_bool(key.as_ref()) {
                Ok(v) => Ok(v),
                Err(e) if e.class() == git2::ErrorClass::Config => {
                    warn!(key=?key.as_ref(), ?default, "bad config value for key, returning default");
                    Ok(default)
                }
                Err(e) => Err(anyhow!(e)),
            }
        }
    }

    fn is_config_key_set<S: AsRef<str>>(&self, key: S) -> Result<bool> {
        // Can't use `get_str` due to https://github.com/rust-lang/git2-rs/issues/474
        match self.get_entry(key.as_ref()) {
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(e) => Err(e.into()),
            Ok(_) => Ok(true),
        }
    }

    fn set_str_if_not_set<S: AsRef<str>>(&mut self, key: S, value: S) -> Result<()> {
        if !self.is_config_key_set(&key)? {
            self.set_str(key.as_ref(), value.as_ref())?;
        }
        Ok(())
    }

    fn dump_config(&self, glob: Option<&str>) -> Result<Vec<(String, String)>> {
        let entries = self.entries(glob)?;
        let mut result: Vec<(String, String)> = Vec::new();

        for entry in entries.into_iter() {
            let entry = entry?;
            match (entry.name(), entry.value()) {
                (Some(name), Some(value)) => result.push((name.to_owned(), value.to_owned())),
                _ => continue,
            }
        }
        Ok(result)
    }

    fn get_i64_with_default<S: AsRef<str>>(&mut self, key: S, default: i64) -> Result<i64> {
        if !self.is_config_key_set(key.as_ref())? {
            Ok(default)
        } else {
            match self.get_i64(key.as_ref()) {
                Ok(v) => Ok(v),
                Err(e) if e.class() == git2::ErrorClass::Config => {
                    warn!(key=?key.as_ref(), ?default, "bad config value for key, returning default");
                    Ok(default)
                }
                Err(e) => Err(anyhow!(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_git_version_parse() -> Result<()> {
        let v = GitVersion::from_str("git version 2.32.5")?;

        assert_eq!(
            GitVersion {
                major: 2,
                minor: 32,
                patch: 5
            },
            v
        );

        let v = GitVersion::from_str("git version 2.32.5-extra-crap-over-here")?;
        assert_eq!(
            GitVersion {
                major: 2,
                minor: 32,
                patch: 5
            },
            v
        );

        let err = GitVersion::from_str("this is garbage");
        assert!(err.is_err());

        Ok(())
    }

    #[test]
    fn test_git_version_current() -> Result<()> {
        // just make sure this doesn't return an error
        let git_binary = GitBinary::for_testing()?;
        let _gv = GitVersion::current(&git_binary).unwrap();
        Ok(())
    }

    static TIMESTAMP: &str = "2022-02-03T00:00:00-05:00";

    #[test]
    fn test_ident_to_signature() -> Result<()> {
        let ident = Ident {
            name: "Arthur Pewtey".to_string(),
            email: "apewtey@twitter.com".to_string(),
            timestamp: FocusTime::parse_from_rfc3339(TIMESTAMP)?,
        };

        let epoch = 1643864400;

        let sig = git2::Signature::new(
            "Arthur Pewtey",
            "apewtey@twitter.com",
            &git2::Time::new(epoch, -5 * 60),
        )?;

        assert_eq!(ident.to_signature()?.to_string(), sig.to_string());
        Ok(())
    }

    #[test]
    fn test_git_binary_from_env() -> Result<()> {
        // just make sure this doesn't barf
        GitBinary::for_testing()?;
        Ok(())
    }

    fn mk_temp_config(content: &str) -> Result<(tempfile::NamedTempFile, git2::Config)> {
        use std::io::prelude::*;

        let temp = tempfile::NamedTempFile::new()?;
        writeln!(temp.as_file(), "{}", content)?;
        let config = git2::Config::open(temp.path())?;

        Ok((temp, config))
    }

    #[test]
    fn test_get_bool_with_default() -> Result<()> {
        let (_temp, mut config) = mk_temp_config(
            r##"
[foo "bar"]
istrue = true
isfalse = false
potato = potato
"##,
        )?;

        assert!(config.get_bool_with_default("foo.bar.istrue", false)?);
        assert!(!config.get_bool_with_default("foo.bar.isfalse", true)?);
        assert!(config.get_bool_with_default("foo.bar.potato", true)?);
        assert!(config.get_bool_with_default("foo.bar.missing", true)?);

        Ok(())
    }

    #[test]
    fn test_get_i64_with_default() -> Result<()> {
        let (_temp, mut config) = mk_temp_config(
            r##"
[foo "bar"]
positive = 1729
negative = -721
potato = potato
"##,
        )?;
        assert_eq!(config.get_i64_with_default("foo.bar.positive", 22)?, 1729);
        assert_eq!(config.get_i64_with_default("foo.bar.negative", 22)?, -721);
        assert_eq!(config.get_i64_with_default("foo.bar.potato", 22)?, 22);
        assert_eq!(config.get_i64_with_default("foo.bar.missing", 22)?, 22);

        Ok(())
    }
}
