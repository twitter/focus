use std::{
    ffi::{OsStr, OsString},
    os::unix::prelude::OsStringExt,
    path::PathBuf,
    process::Stdio,
    str::FromStr,
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;
use std::process::Command;

use crate::{
    app::App,
    util::{
        sandbox_command::{SandboxCommand, SandboxCommandOutput},
        time::GitIdentTime,
    },
};

use super::time::{FocusTime, GitTime};

pub fn git_binary() -> OsString {
    OsString::from("git")
}

/// resolves the git binary in PATH
pub fn git_binary_path() -> Result<PathBuf> {
    Ok(which::which(&git_binary())?)
}

const NL: u8 = b'\n';

pub fn git_exec_path(git_binary_path: &Path) -> Result<PathBuf> {
    let mut output = Command::new(git_binary_path)
        .arg("--exec-path")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() {
        bail!("git --exec-path failed to run");
    }

    let stdout = &mut output.stdout;
    if *stdout.last().unwrap() == NL {
        stdout.pop();
    }

    let out = OsString::from_vec(output.stdout);
    Ok(PathBuf::from(out).canonicalize()?)
}

pub fn git_command<S: AsRef<str>>(
    description: S,
    app: Arc<App>,
) -> Result<(Command, SandboxCommand)> {
    SandboxCommand::new(description.as_ref().to_owned(), git_binary(), app)
}

pub fn remote_add<P: AsRef<Path>>(
    repo_path: P,
    name: &str,
    url: &OsStr,
    app: Arc<App>,
) -> Result<()> {
    let description = format!("Adding remote {} ({})", &name, &url.to_string_lossy());
    let (mut cmd, scmd) = git_command(description, app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path)
            .arg("remote")
            .arg("add")
            .arg(name)
            .arg(url),
        SandboxCommandOutput::Stderr,
        "git remote add",
    )
    .map(|_| ())
}

pub fn write_config<P: AsRef<Path>>(
    repo_path: P,
    key: &str,
    val: &str,
    app: Arc<App>,
) -> Result<()> {
    let description = format!("Setting Git config {} {}", key, val);
    let (mut cmd, scmd) = git_command(description, app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).arg("config").arg(key).arg(val),
        SandboxCommandOutput::Stderr,
        "git config (write)",
    )
    .map(|_| ())
}

pub fn read_config<P: AsRef<Path>>(
    repo_path: P,
    key: &str,
    app: Arc<App>,
) -> Result<Option<String>> {
    let description = format!("Reading Git config {}", key);
    if let Ok(result) = run_consuming_stdout(description, repo_path, &["config", key], app) {
        return Ok(Some(result));
    }

    Ok(None)
}

pub fn unset_config<P: AsRef<Path>>(repo_path: P, key: &str, app: Arc<App>) -> Result<()> {
    let description = format!("git config --unset {}", key);
    let (mut cmd, _scmd) = git_command(description, app)?;
    cmd.arg("config")
        .arg("--unset")
        .arg(key)
        .current_dir(repo_path)
        .status()
        .with_context(|| format!("Running `git config --unset {}` failed", key))?;
    Ok(())
}

pub fn run_consuming_stdout<S, P, I, O>(
    description: S,
    repo: P,
    args: I,
    app: Arc<App>,
) -> Result<String>
where
    S: AsRef<str>,
    P: AsRef<Path>,
    I: IntoIterator<Item = O>,
    O: AsRef<OsStr>,
{
    let (mut cmd, scmd) = git_command(description, app)?;
    if let Err(e) = cmd.current_dir(repo).args(args).status() {
        scmd.log(SandboxCommandOutput::Stderr, "git command")?;
        bail!("git command failed: {}", e);
    }
    let mut stdout_contents = String::new();
    scmd.read_to_string(SandboxCommandOutput::Stdout, &mut stdout_contents)?;
    Ok(stdout_contents.trim().to_owned())
}

pub fn find_top_level(app: Arc<App>, path: &Path) -> Result<PathBuf> {
    if let Ok(path) = std::fs::canonicalize(path) {
        Ok(PathBuf::from(
            run_consuming_stdout(
                format!("Finding top level of repo in {}", path.display()),
                path,
                &["rev-parse", "--show-toplevel"],
                app,
            )
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
    run_consuming_stdout(
        format!("Determining the current commit in repo {}", repo.display()),
        repo,
        &["rev-parse", "HEAD"],
        app,
    )
}

pub fn get_current_branch(app: Arc<App>, repo: &Path) -> Result<String> {
    run_consuming_stdout(
        format!("Determining the current branch in repo {}", repo.display()),
        repo,
        &["branch", "--show-current"],
        app,
    )
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
                app.ui().log(
                    "Branch Switch",
                    format!(
                        "Couldn't determine the current branch in {}, using the default 'master'.",
                        repo.display()
                    ),
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
        let attachment_description = if detach { "detached" } else { "attached" };
        let description = format!(
            "Switching to {} in {} ({})",
            &branch_name,
            self.repo.display(),
            attachment_description,
        );
        let (mut cmd, scmd) = git_command(description.clone(), self.app.clone())?;
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
        scmd.ensure_success_or_log(cmd, SandboxCommandOutput::Stderr, &description)?;

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
    pub fn current() -> Result<GitVersion> {
        let out = Command::new(git_binary())
            .arg("version")
            .stderr(Stdio::inherit())
            .output()?;

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
        let _gv = GitVersion::current().unwrap();
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
    fn test_git_binary_path() -> Result<()> {
        // just make sure this doesn't barf
        git_binary_path()?;
        Ok(())
    }

    #[test]
    fn test_git_exec_path() -> Result<()> {
        // just make sure this doesn't barf
        git_exec_path(&git_binary_path()?)?;
        Ok(())
    }
}
