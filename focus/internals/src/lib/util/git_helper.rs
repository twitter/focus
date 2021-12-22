use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::{
    app::App,
    util::sandbox_command::{SandboxCommand, SandboxCommandOutput},
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
    if let Ok(result) =
        run_consuming_stdout(description, repo_path, vec!["config", key], app.clone())
    {
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

pub fn run_consuming_stdout<P, I, S>(
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
        scmd.log(SandboxCommandOutput::Stderr, &"git command")?;
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

pub fn get_current_revision(app: Arc<App>, repo: &Path) -> Result<String> {
    run_consuming_stdout(
        format!("Determining the current commit in repo {}", repo.display()),
        repo,
        vec!["rev-parse", "HEAD"],
        app,
    )
}

pub fn get_current_branch(app: Arc<App>, repo: &Path) -> Result<String> {
    run_consuming_stdout(
        format!("Determining the current branch in repo {}", repo.display()),
        repo,
        vec!["branch", "--show-current"],
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
    #[allow(unused)]
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
        repo: PathBuf,
        refname: String,
        alternate: Option<PathBuf>,
    ) -> Result<Self> {
        let current_branch = {
            let hint = get_current_branch(app.clone(), repo.as_path())?
                .trim()
                .to_owned();
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
            repo,
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
            self.switch(&refname_to_switch_back_to, false)
                .expect("Switching back to the original branch failed");
        }
    }
}

/// The state of a repository encompassing its origin URL and current commit ID.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct RepoState {
    origin_url: String,
    commit_id: String,
}

impl RepoState {
    pub fn new(repo_path: &dyn AsRef<Path>, app: Arc<App>) -> Result<Self> {
        let origin_url = run_consuming_stdout(
            "Reading origin URL".to_owned(),
            repo_path,
            vec!["remote", "get-url", "origin"],
            app.clone(),
        )
        .context("Failed to determine the origin URL")?;

        let commit_id = run_consuming_stdout(
            "Determining commit ID".to_owned(),
            repo_path,
            vec!["rev-parse", "HEAD"],
            app.clone(),
        )
        .context("Failed to determine the commit ID")?;

        Ok(Self {
            origin_url,
            commit_id,
        })
    }
}

impl Display for RepoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}?@={}", self.origin_url, self.commit_id)
    }
}
