// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use anyhow::{Context, Result};
use console::style;
use focus_util::{app::App, paths::is_relevant_to_build_graph};
use git2::{FileMode, TreeWalkMode, TreeWalkResult};
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

enum SkimProjectOrTarget {
    Project { name: String, project: Project },
    BazelPackage { name: String, build_file: PathBuf },
}

impl SkimItem for SkimProjectOrTarget {
    fn text(&self) -> std::borrow::Cow<str> {
        match self {
            SkimProjectOrTarget::Project { name, project } => {
                Cow::Owned(format!("{} - {}", name, project.description))
            }
            SkimProjectOrTarget::BazelPackage {
                name,
                build_file: _,
            } => Cow::Owned(format!("bazel://{name}")),
        }
    }

    fn display<'a>(&'a self, _context: skim::DisplayContext<'a>) -> skim::AnsiString<'a> {
        match self {
            SkimProjectOrTarget::Project { name, project } => {
                let display = format!("{} - {}", style(name).bold(), project.description);
                AnsiString::parse(&display)
            }
            SkimProjectOrTarget::BazelPackage {
                name,
                build_file: _,
            } => {
                let display = style(format!("bazel://{name}")).bold().to_string();
                AnsiString::parse(&display)
            }
        }
    }

    fn preview(&self, _context: skim::PreviewContext) -> skim::ItemPreview {
        match self {
            SkimProjectOrTarget::Project { name: _, project } => {
                let targets: Vec<String> = project
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
            SkimProjectOrTarget::BazelPackage {
                name: _,
                build_file,
            } => {
                let preview = format!(
                    "\
{title}

Press <space> to select/unselect this package.
Press <enter> to apply changes.

BUILD file: {build_file}
",
                    title = self.text(),
                    build_file = build_file.display(),
                );
                skim::ItemPreview::Text(preview)
            }
        }
    }
}

fn spawn_target_search_thread(tx: SkimItemSender, sparse_repo_path: PathBuf) {
    fn inner(tx: SkimItemSender, sparse_repo_path: PathBuf) -> anyhow::Result<()> {
        let repo = git2::Repository::open(sparse_repo_path)?;
        let head_ref = repo.head()?;
        let head_tree = head_ref.peel_to_tree()?;

        head_tree.walk(TreeWalkMode::PreOrder, |root, entry| {
            if root.is_empty() {
                return TreeWalkResult::Ok;
            }

            let entry_name = match entry.name() {
                Some(name) => name,
                None => return TreeWalkResult::Skip,
            };
            if (entry.filemode() == i32::from(FileMode::Blob)
                || entry.filemode() == i32::from(FileMode::BlobExecutable))
                && is_relevant_to_build_graph(entry_name)
            {
                let name = root.trim_matches('/').to_string();
                let build_file = PathBuf::from(root).join(entry_name);
                let item = SkimProjectOrTarget::BazelPackage { name, build_file };
                if tx.send(Arc::new(item)).is_err() {
                    return TreeWalkResult::Abort;
                }
            }
            TreeWalkResult::Ok
        })?;

        Ok(())
    }

    thread::spawn(move || {
        if let Err(err) = inner(tx, sparse_repo_path) {
            info!(?err, "Error while searching for targets");
        }
    });
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
            let item = SkimProjectOrTarget::Project {
                name: name.clone(),
                project: project.clone(),
            };
            skim_tx
                .send(Arc::new(item))
                .context("Sending item to skim")?;
        }

        spawn_target_search_thread(skim_tx, sparse_repo.as_ref().to_path_buf());
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
            .map(|item| item.as_any().downcast_ref::<SkimProjectOrTarget>().unwrap())
            .map(|item| match item {
                SkimProjectOrTarget::Project { name, project: _ } => name.clone(),
                SkimProjectOrTarget::BazelPackage {
                    name,
                    build_file: _,
                } => format!("bazel://{name}/..."),
            })
            .collect();
        add(sparse_repo, true, selected_projects, app)?;
    }

    Ok(())
}
