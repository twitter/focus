use anyhow::Result;
use focus_internals::model::repo::Repo;
use focus_util::app::{App, ExitCode};
use std::{path::Path, sync::Arc};

pub fn run(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<ExitCode> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    let selection = selections.selection()?;
    println!("{}", selection);

    Ok(ExitCode(0))
}
