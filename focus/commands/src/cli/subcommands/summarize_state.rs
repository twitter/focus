use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::git_helper;
use crate::{sandbox::Sandbox, sparse_repos::Spec};

pub fn run(sandbox: &Sandbox, repo: &PathBuf) -> Result<()> {
    let sync_state = git_helper::read_config(repo.as_path(), "twitter.focus.sync_point", sandbox)?;

    let (mut cmd, scmd) = git_helper::git_command(sandbox)?;
    let status = cmd.current_dir(repo.as_path()).arg("diff").arg("--name-only");
  
    // if let Err(e) = cmd.current_dir(repo).arg("").status() {

    // }

    Ok(())
}
