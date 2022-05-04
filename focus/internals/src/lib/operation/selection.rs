use std::{
    fmt::{Debug, Formatter},
    io::Write,
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result};
use focus_util::app::App;
use tracing::info;

use crate::model::{repo::Repo, selection::*};

fn mutate(
    sparse_repo: &dyn AsRef<Path>,
    sync_if_changed: bool,
    disposition: Disposition,
    projects_and_targets: Vec<String>,
    app: Arc<focus_util::app::App>,
) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let mut selections = Selections::try_from(&repo)?;
    if selections.mutate(disposition, &projects_and_targets)? {
        selections.save().context("Saving selection")?;
        if sync_if_changed {
            info!("Synchronizing after selection changed");
            super::sync::run(sparse_repo.as_ref(), app.clone())?;
        }
    }

    Ok(())
}

pub fn add(
    sparse_repo: &dyn AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    mutate(
        sparse_repo,
        sync_if_changed,
        Disposition::Add,
        projects_and_targets,
        app,
    )
}

pub fn remove(
    sparse_repo: &dyn AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    mutate(
        sparse_repo,
        sync_if_changed,
        Disposition::Remove,
        projects_and_targets,
        app,
    )
}

pub fn status(sparse_repo: &dyn AsRef<Path>, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let selections = Selections::try_from(&repo)?;
    let selection = selections.computed_selection()?;
    println!("{}", selection);
    Ok(())
}

pub fn list_projects(sparse_repo: &dyn AsRef<Path>, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let selections = Selections::try_from(&repo)?;
    println!("{}", selections.optional_projects);
    Ok(())
}
