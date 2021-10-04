use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::{
    app::App,
    sandbox_command::{SandboxCommand, SandboxCommandOutput},
};

pub fn git_binary() -> OsString {
    OsString::from("git")
}

pub fn git_command(description: String, app: Arc<App>) -> Result<(Command, SandboxCommand)> {
    SandboxCommand::new(description, git_binary(), app)
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
    let description = format!("git config {} {}", key, val);
    let (mut cmd, scmd) = git_command(description, app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).arg("config").arg(key).arg(val),
        SandboxCommandOutput::Stderr,
        "git config (write)",
    )
    .map(|_| ())
}

pub fn read_config<P: AsRef<Path>>(repo_path: P, key: &str, app: Arc<App>) -> Result<String> {
    let description = format!("git config {}", key);
    let (mut cmd, scmd) = git_command(description, app)?;
    let mut output_string = String::new();
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).arg("config").arg(key),
        SandboxCommandOutput::Stderr,
        "git config (read)",
    )?;
    scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)
        .with_context(|| format!("reading config key {}", key))?;
    Ok(output_string)
}

pub fn unset_config<P: AsRef<Path>>(_repo_path: P, key: &str, app: Arc<App>) -> Result<()> {
    let description = format!("git config --unset {}", key);
    let (mut cmd, _scmd) = git_command(description, app)?;
    cmd.arg("config")
        .arg("--unset")
        .arg(key)
        .status()
        .with_context(|| format!("Running `git config --unset {}` failed", key))?;
    Ok(())
}

pub fn run_git_command_consuming_stdout<P, I, S>(
    description: String,
    repo: P,
    args: I,
    app: Arc<App>,
) -> Result<String>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (mut cmd, scmd) = git_command(description, app)?;
    if let Err(e) = cmd.current_dir(repo).args(args).status() {
        scmd.log(
            crate::sandbox_command::SandboxCommandOutput::Stderr,
            &"git command",
        )?;
        bail!("git command failed: {}", e);
    }
    let mut stdout_contents = String::new();
    scmd.read_to_string(SandboxCommandOutput::Stdout, &mut stdout_contents)?;
    Ok(stdout_contents.trim().to_owned())
}

pub fn find_top_level(app: Arc<App>, path: &Path) -> Result<PathBuf> {
    if let Ok(path) = std::fs::canonicalize(path) {
        Ok(PathBuf::from(
            run_git_command_consuming_stdout(
                "Finding the repo's top level".to_owned(),
                path,
                vec!["rev-parse", "--show-toplevel"],
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
