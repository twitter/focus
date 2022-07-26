// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, path::Path, sync::Arc};

use anyhow::{Context, Result};
use console::style;
use focus_util::app::App;
use skim::{
    prelude::SkimOptionsBuilder, AnsiString, Skim, SkimItem, SkimItemReceiver, SkimItemSender,
};
use tracing::info;

use focus_internals::model::{repo::Repo, selection::*};

fn mutate(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    action: OperationAction,
    projects_and_targets: Vec<String>,
    app: Arc<focus_util::app::App>,
) -> Result<bool> {
    let mut synced = false;
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let mut selections = repo.selection_manager().context("Loading the selection")?;
    let backup = selections
        .create_backup()
        .context("Creating a backup of the current selection")?;
    if selections
        .mutate(action, &projects_and_targets)
        .context("Updating the selection")?
    {
        selections.save().context("Saving selection")?;
        if sync_if_changed {
            info!("Synchronizing after selection changed");
            let result = super::sync::run(sparse_repo.as_ref(), crate::sync::SyncMode::Normal, app)
                .context("Synchronizing changes")?;
            synced = result.status == super::sync::SyncStatus::Success;
            backup.discard();
        }
    }

    Ok(synced)
}

pub fn add(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    app: Arc<App>,
) -> Result<bool> {
    mutate(
        sparse_repo,
        sync_if_changed,
        OperationAction::Add,
        projects_and_targets,
        app,
    )
}

pub fn remove(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    app: Arc<App>,
) -> Result<bool> {
    mutate(
        sparse_repo,
        sync_if_changed,
        OperationAction::Remove,
        projects_and_targets,
        app,
    )
}

pub fn list_projects(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    println!("{}", selections.project_catalog().optional_projects);
    Ok(())
}

struct SkimProject {
    name: String,
    project: Project,
}

impl SkimItem for SkimProject {
    fn text(&self) -> std::borrow::Cow<str> {
        Cow::Owned(format!("{} - {}", self.name, self.project.description))
    }

    fn display<'a>(&'a self, _context: skim::DisplayContext<'a>) -> skim::AnsiString<'a> {
        let display = format!(
            "{} - {}",
            style(&self.name).bold(),
            self.project.description
        );
        AnsiString::parse(&display)
    }

    fn preview(&self, _context: skim::PreviewContext) -> skim::ItemPreview {
        let targets: Vec<String> = self
            .project
            .targets
            .iter()
            .map(|target| format!("- {}\n", target))
            .collect();
        let preview = format!(
            "\
{title}

Press <space> to select/unselect this project.
Press <enter> to apply changes.

Includes {num_targets} target(s):
{targets}
",
            title = self.text(),
            num_targets = targets.len(),
            targets = targets.join("")
        );
        skim::ItemPreview::Text(preview)
    }
}

pub fn add_interactive(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let selections = repo.selection_manager()?;

    let options = SkimOptionsBuilder::default()
        .multi(true)
        .bind(vec!["Space:toggle"])
        .preview(Some("Project description"))
        .build()
        .map_err(|err| anyhow::anyhow!("{}", err))?;

    let skim_rx = {
        let (skim_tx, skim_rx): (SkimItemSender, SkimItemReceiver) = skim::prelude::unbounded();
        for (name, project) in selections
            .project_catalog()
            .optional_projects
            .underlying
            .iter()
        {
            let item = SkimProject {
                name: name.clone(),
                project: project.clone(),
            };
            skim_tx
                .send(Arc::new(item))
                .context("Sending item to skim")?;
        }
        skim_rx
    };

    let skim_output = Skim::run_with(&options, Some(skim_rx))
        .ok_or_else(|| anyhow::anyhow!("Failed to select items"))?;
    if skim_output.is_abort {
        info!("Aborted by user.");
    } else {
        let selected_projects: Vec<String> = skim_output
            .selected_items
            .iter()
            .map(|item| item.as_any().downcast_ref::<SkimProject>().unwrap())
            .map(|project| project.name.clone())
            .collect();
        add(sparse_repo, true, selected_projects, app)?;
    }

    Ok(())
}
