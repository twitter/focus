// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Cow,
    collections::HashSet,
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use anyhow::{bail, Context, Result};
use console::style;
use focus_util::{
    app::App,
    git_helper::{get_changed_paths_between_trees, get_head_commit},
    paths::is_relevant_to_build_graph,
};
use git2::{FileMode, TreeWalkMode, TreeWalkResult};
use skim::{
    prelude::SkimOptionsBuilder, AnsiString, Skim, SkimItem, SkimItemReceiver, SkimItemSender,
};

use tracing::info;

use focus_internals::{
    model::{repo::Repo, selection::*},
    target::Target,
};

use crate::sync::SyncMode;

pub fn save(
    sparse_repo: impl AsRef<Path>,
    project_name: String,
    project_file: Option<String>,
    project_description: Option<String>,
    app: Arc<focus_util::app::App>,
) -> Result<bool> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let mut selection_manager = repo.selection_manager().context("Loading the selection")?;
    let project_found = selection_manager
        .project_catalog()
        .optional_projects
        .underlying
        .get(&project_name);
    let project_description = match project_found {
        Some(p) => Some(p.description.clone()),
        None => match project_description {
            Some(..) => project_description,
            None => None,
        },
    };
    if project_description.is_none() {
        panic!("Project {} not found and no project description provided. Please provide a project description.", project_name);
    }
    let project = Project {
        name: project_name.clone(),
        description: project_description.unwrap(),
        mandatory: false,
        targets: selection_manager
            .selection()?
            .targets
            .into_iter()
            .map(|t| t.to_string())
            .collect(),
        projects: selection_manager
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect(),
    };
    selection_manager
        .mut_project_catalog()
        .set_project(project_name, project, project_file)
        .unwrap();

    Ok(true)
}

fn mutate(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    action: OperationAction,
    projects_and_targets: Vec<String>,
    unroll: bool,
    app: Arc<focus_util::app::App>,
) -> Result<bool> {
    let mut synced = false;
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let mut selections = repo.selection_manager().context("Loading the selection")?;
    let backup = if sync_if_changed {
        Some(
            selections
                .create_backup()
                .context("Creating a backup of the current selection")?,
        )
    } else {
        None
    };

    let mut projects_and_targets = projects_and_targets;
    
    if unroll {
        let mut projects = vec![];
        let mut targets = vec![];

        match action {
            OperationAction::Add => {
                for i in projects_and_targets.clone() {
                    if Target::try_from(i.as_str()).is_ok() {
                        targets.push(i);
                    } else {
                        projects.extend(
                            selections
                                .project_catalog()
                                .optional_projects
                                .underlying
                                .get(&i)
                                .with_context(|| {
                                    format!("Couldn't find project definition for {}.", i)
                                })?
                                .projects
                                .clone(),
                        );
                        targets.extend(
                            selections
                                .project_catalog()
                                .optional_projects
                                .underlying
                                .get(&i)
                                .with_context(|| {
                                    format!("Couldn't find project definition for {}.", i)
                                })?
                                .targets
                                .clone(),
                        );
                    };
                }
            }
            OperationAction::Remove => {
                projects = selections
                    .selection()?
                    .projects
                    .into_iter()
                    .map(|x| x.name)
                    .collect();
                targets = selections
                    .selection()?
                    .targets
                    .into_iter()
                    .map(|x| match x {
                        Target::Bazel(c) => format!("bazel:{}", c),
                        Target::Directory(c) => format!("bazel:{}", c),
                        Target::Pants(c) => format!("bazel:{}", c),
                    })
                    .collect();
            }
        }

        projects_and_targets = targets;
        projects_and_targets.extend(projects);
    }

    if selections
        .mutate(action, &projects_and_targets)
        .context("Updating the selection")?
    {
        selections.save().context("Saving selection")?;
        if sync_if_changed {
            info!("Synchronizing after selection changed");
            // TODO: Use the correct sync mode here. Sync will override for SyncMode::Incremental, but that feels janky.
            let result = super::sync::run(sparse_repo.as_ref(), SyncMode::Incremental, app)
                .context("Synchronizing changes")?;
            synced = result.status == super::sync::SyncStatus::Success;
            backup.unwrap().discard();
        }
    }

    Ok(synced)
}

pub fn add(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    unroll: bool,
    app: Arc<App>,
) -> Result<bool> {
    mutate(
        sparse_repo,
        sync_if_changed,
        OperationAction::Add,
        projects_and_targets,
        unroll,
        app,
    )
}

pub fn remove(
    sparse_repo: impl AsRef<Path>,
    sync_if_changed: bool,
    projects_and_targets: Vec<String>,
    all: bool,
    app: Arc<App>,
) -> Result<bool> {
    mutate(
        sparse_repo,
        sync_if_changed,
        OperationAction::Remove,
        projects_and_targets,
        all,
        app,
    )
}

pub fn list_projects(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    println!("{}", selections.project_catalog().optional_projects);
    Ok(())
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum SkimSource {
    Project,
    Phabricator,
    Repository,
    CommitHistory,
}

impl Display for SkimSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkimSource::Project => {
                write!(f, "project")
            }
            SkimSource::Phabricator => {
                write!(f, "Phabricator")
            }
            SkimSource::Repository => {
                write!(f, "repository")
            }
            SkimSource::CommitHistory => {
                write!(f, "your commits")
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum SkimProjectOrTarget {
    Project {
        source: SkimSource,
        name: String,
        project: Project,
    },
    BazelPackage {
        source: SkimSource,
        name: String,
        build_file: PathBuf,
    },
}

impl SkimItem for SkimProjectOrTarget {
    fn text(&self) -> std::borrow::Cow<str> {
        match self {
            SkimProjectOrTarget::Project {
                source: _,
                name,
                project,
            } => Cow::Owned(format!("{} - {}", name, project.description)),
            SkimProjectOrTarget::BazelPackage {
                source: _,
                name,
                build_file: _,
            } => Cow::Owned(format!("bazel://{name}")),
        }
    }

    fn display<'a>(&'a self, _context: skim::DisplayContext<'a>) -> skim::AnsiString<'a> {
        let display_text = match self {
            SkimProjectOrTarget::Project {
                source,
                name,
                project,
            } => {
                format!(
                    "(from {source}) {name} - {description}",
                    source = style(source).yellow(),
                    name = style(name).bold(),
                    description = project.description
                )
            }

            SkimProjectOrTarget::BazelPackage {
                source,
                name,
                build_file: _,
            } => {
                format!(
                    "(from {source}) {name}",
                    source = style(source).yellow(),
                    name = style(format!("bazel://{name}")).bold()
                )
            }
        };
        AnsiString::parse(&display_text)
    }

    fn preview(&self, _context: skim::PreviewContext) -> skim::ItemPreview {
        match self {
            SkimProjectOrTarget::Project {
                source,
                name: _,
                project,
            } => {
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

Source: {source}
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
                source,
                name: _,
                build_file,
            } => {
                let preview = format!(
                    "\
{title}

Press <space> to select/unselect this package.
Press <enter> to apply changes.

Source: {source}
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
                let item = SkimProjectOrTarget::BazelPackage {
                    source: SkimSource::Repository,
                    name,
                    build_file,
                };
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

fn suggest_skim_item_from_path(
    source: SkimSource,
    head_tree: &git2::Tree,
    path: &Path,
) -> Option<SkimProjectOrTarget> {
    let build_file_patterns = &["BUILD", "BUILD.bazel"];
    let mut base_path: &Path = path;
    loop {
        base_path = match base_path.parent() {
            Some(parent) => parent,
            None => return None,
        };

        let build_file = match build_file_patterns.iter().find_map(|file_name| {
            let path = base_path.join(file_name);
            match head_tree.get_path(&path) {
                Ok(_) => Some(path),
                Err(_) => None,
            }
        }) {
            Some(build_file) => build_file,
            None => continue,
        };

        let package_name = match build_file.parent().and_then(|path| path.to_str()) {
            Some(package_name) => package_name,
            None => continue,
        };

        // The root package probably has a `BUILD` file, but we shouldn't
        // suggest that, as it would negate the advantages of a sparse checkout.
        if !package_name.is_empty() {
            return Some(SkimProjectOrTarget::BazelPackage {
                source,
                name: package_name.to_owned(),
                build_file,
            });
        }
    }
}

fn query_phabricator_for_recently_touched_paths() -> Result<Vec<PathBuf>> {
    use focus_platform::phabricator::*;
    let response = query::<user_whoami::Endpoint>(user_whoami::Request {})
        .context("Querying Phabricator whoami")?;

    let response = query::<differential_query::Endpoint>(differential_query::Request {
        authors: Some(vec![response.phid]),
        limit: Some(100),
        ..Default::default()
    })
    .context("Querying recent revisions")?;

    let paths =
        query::<differential_changeset_search::Endpoint>(differential_changeset_search::Request {
            constraints: differential_changeset_search::Constraints {
                diffPHIDs: Some(
                    response
                        .0
                        .iter()
                        .filter_map(|x| x.activeDiffPHID.clone())
                        .collect(),
                ),
            },
        })?;
    Ok(paths
        .data
        .into_iter()
        .map(|item| PathBuf::from(item.fields.path.displayPath))
        .collect())
}

#[cfg(feature = "twttr")]
#[test]
fn test_query_phabricator() -> Result<()> {
    let paths = query_phabricator_for_recently_touched_paths()?;
    assert_ne!(
        paths,
        Vec::<PathBuf>::new(),
        "\
Could not query Phabricator for recently-touched paths on behalf of the user \
running this test. There might be a legitimate bug in the code, but it's also \
possible that there is no authenticated user (is on a CI machine, or `arc` \
isn't configured) or because the authenticated user really has no \
recently-touched paths.
"
    );
    Ok(())
}

fn spawn_phabricator_query_thread(tx: SkimItemSender, sparse_repo_path: PathBuf) {
    fn inner(tx: SkimItemSender, sparse_repo_path: PathBuf) -> Result<()> {
        let repo = git2::Repository::open(sparse_repo_path)?;
        let head_ref = repo.head()?;
        let head_tree = head_ref.peel_to_tree()?;

        let paths = query_phabricator_for_recently_touched_paths()?;

        // The order might be meaningful (e.g. most recent paths listed first),
        // so traverse paths in order rather than collecting into a `HashSet`
        // immediately.
        let mut seen_items = HashSet::new();
        for path in paths {
            if let Some(item) =
                suggest_skim_item_from_path(SkimSource::Phabricator, &head_tree, &path)
            {
                if seen_items.insert(item.clone()) {
                    tx.send(Arc::new(item))?;
                }
            }
        }

        Ok(())
    }

    thread::spawn(move || {
        if let Err(err) = inner(tx, sparse_repo_path) {
            info!(?err, "Error while querying Phabricator");
        }
    });
}

fn spawn_commit_history_search_thread(tx: SkimItemSender, sparse_repo_path: PathBuf) {
    fn inner(tx: SkimItemSender, sparse_repo_path: PathBuf) -> Result<()> {
        let repo = git2::Repository::open(sparse_repo_path)?;
        let head_commit = get_head_commit(&repo)?;
        let head_tree = head_commit.tree().context("Getting HEAD tree")?;
        let user_email = repo
            .config()
            .context("Getting config")?
            .get_string("user.email")
            .context("Reading user.email")?;

        let mut seen_items = HashSet::new();
        let mut commit = Some(head_commit);
        while let Some(current_commit) = commit {
            let parent_commit = match current_commit.parent(0) {
                Ok(commit) => Some(commit),
                Err(err) if err.code() == git2::ErrorCode::NotFound => None,
                Err(err) => bail!("Failed to get parent commit: {err}"),
            };
            let parent_tree = match &parent_commit {
                Some(parent_commit) => Some(parent_commit.tree()?),
                None => None,
            };

            if current_commit.author().email_bytes() == user_email.as_bytes() {
                let changed_paths = get_changed_paths_between_trees(
                    &repo,
                    parent_tree.as_ref(),
                    Some(&current_commit.tree()?),
                )?;

                let mut items = changed_paths
                    .into_iter()
                    .filter_map(|path| {
                        let path = path.parent()?;
                        suggest_skim_item_from_path(SkimSource::CommitHistory, &head_tree, path)
                    })
                    .collect::<Vec<_>>();
                items.sort();
                for item in items {
                    if seen_items.insert(item.clone()) {
                        tx.send(Arc::new(item))?;
                    }
                }
            }

            commit = parent_commit;
        }

        Ok(())
    }

    thread::spawn(move || {
        if let Err(err) = inner(tx, sparse_repo_path) {
            info!(?err, "Error while querying Phabricator");
        }
    });
}

pub fn add_interactive(
    sparse_repo: impl AsRef<Path>,
    app: Arc<App>,
    search_all_targets: bool,
    unroll: bool,
) -> Result<()> {
    let sparse_repo_path = sparse_repo.as_ref();
    let repo = Repo::open(sparse_repo_path, app.clone())?;
    let selections = repo.selection_manager()?;

    let options = SkimOptionsBuilder::default()
        .multi(true)
        .bind(vec!["Space:toggle"])
        .preview(Some("Project description"))
        .reverse(true)
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
                source: SkimSource::Project,
                name: name.clone(),
                project: project.clone(),
            };
            skim_tx
                .send(Arc::new(item))
                .context("Sending item to skim")?;
        }

        if search_all_targets {
            spawn_target_search_thread(skim_tx.clone(), sparse_repo_path.to_path_buf());
        }
        spawn_phabricator_query_thread(skim_tx.clone(), sparse_repo_path.to_path_buf());
        spawn_commit_history_search_thread(skim_tx, sparse_repo_path.to_path_buf());
        skim_rx
    };

    let skim_output = Skim::run_with(&options, Some(skim_rx))
        .ok_or_else(|| anyhow::anyhow!("Failed to select items"))?;
    if skim_output.is_abort {
        info!("Aborted by user.");
        if !search_all_targets {
            println!("Didn't find what you were looking for? You can search all projects by passing the --all flag.");
        }
    } else {
        let selected_projects: Vec<String> = skim_output
            .selected_items
            .iter()
            .map(|item| item.as_any().downcast_ref::<SkimProjectOrTarget>().unwrap())
            .map(|item| match item {
                SkimProjectOrTarget::Project {
                    source: _,
                    name,
                    project: _,
                } => name.clone(),
                SkimProjectOrTarget::BazelPackage {
                    source: _,
                    name,
                    build_file: _,
                } => format!("bazel://{name}/..."),
            })
            .collect();
        add(sparse_repo, true, selected_projects, unroll, app)?;
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use anyhow::Result;

    use crate::testing::integration::RepoPairFixture;

    #[test]
    fn selection_add_unroll() -> Result<()> {
        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let projects = vec![String::from("team_zissou/project_c")];

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            projects,
            true,
            fixture.app.clone(),
        )?;
        let project_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            project_names,
            HashSet::from(["team_banzai/project_a".to_string()])
        );
        let target_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .targets
            .into_iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(
            target_names,
            HashSet::from(["bazel://project_b/...".to_string()])
        );

        Ok(())
    }

    #[test]
    fn selection_remove_all() -> Result<()> {
        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let projects = vec![String::from("team_zissou/project_c")];

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            projects.clone(),
            true,
            fixture.app.clone(),
        )?;

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            Vec::new(),
            true,
            fixture.app.clone(),
        )?;

        let project_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(project_names, HashSet::new());

        let target_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .targets
            .into_iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(target_names, HashSet::new());
        Ok(())
    }

    #[test]
    fn selection_save_new() -> Result<()> {
        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let projects = vec![String::from("team_zissou/project_c")];

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            projects,
            true,
            fixture.app.clone(),
        )?;
        let project_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            project_names,
            HashSet::from(["team_banzai/project_a".to_string()])
        );
        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            vec!["bazel://project_a/...".to_string()],
            true,
            fixture.app.clone(),
        )?;
        crate::selection::save(
            &fixture.sparse_repo_path,
            "team_zissou/project_c_2".to_string(),
            Some("project_c".to_string()),
            Some("project c2".to_string()),
            fixture.app.clone(),
        )?;
        assert_eq!(
            fs::read_to_string(
                &fixture
                    .sparse_repo_path
                    .join("focus/projects/project_c.projects.json")
            )?,
            r#"{
    "projects": [
        {
            "name": "team_zissou/project_c",
            "description": "Stuff relating to project C",
            "targets": [
                "bazel://project_b/..."
            ],
            "projects": [
                "team_banzai/project_a"
            ]
        },
        {
            "name": "team_zissou/project_c_2",
            "description": "project c2",
            "targets": [
                "bazel://project_a/...",
                "bazel://project_b/..."
            ],
            "projects": [
                "team_banzai/project_a"
            ]
        }
    ]
}"#
        );
        Ok(())
    }

    #[test]
    fn selection_save_new_file() -> Result<()> {
        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let projects = vec![String::from("team_zissou/project_c")];

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            projects,
            true,
            fixture.app.clone(),
        )?;
        let project_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            project_names,
            HashSet::from(["team_banzai/project_a".to_string()])
        );
        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            vec!["bazel://project_a/...".to_string()],
            true,
            fixture.app.clone(),
        )?;
        crate::selection::save(
            &fixture.sparse_repo_path,
            "team_zissou/project_d".to_string(),
            Some("project_d".to_string()),
            Some("project d".to_string()),
            fixture.app.clone(),
        )?;
        assert_eq!(
            fs::read_to_string(
                &fixture
                    .sparse_repo_path
                    .join("focus/projects/project_d.projects.json")
            )?,
            r#"{
    "projects": [
        {
            "name": "team_zissou/project_d",
            "description": "project d",
            "targets": [
                "bazel://project_a/...",
                "bazel://project_b/..."
            ],
            "projects": [
                "team_banzai/project_a"
            ]
        }
    ]
}"#
        );
        Ok(())
    }

    #[test]
    fn selection_save_existing() -> Result<()> {
        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let projects = vec![String::from("team_zissou/project_c")];

        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            projects,
            true,
            fixture.app.clone(),
        )?;
        let project_names: HashSet<String> = fixture
            .sparse_repo()?
            .selection_manager()?
            .selection()?
            .projects
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            project_names,
            HashSet::from(["team_banzai/project_a".to_string()])
        );
        crate::selection::add(
            &fixture.sparse_repo_path,
            true,
            vec!["bazel://project_a/...".to_string()],
            true,
            fixture.app.clone(),
        )?;
        crate::selection::save(
            &fixture.sparse_repo_path,
            "team_zissou/project_c".to_string(),
            None,
            None,
            fixture.app.clone(),
        )?;
        assert_eq!(
            fs::read_to_string(
                &fixture
                    .sparse_repo_path
                    .join("focus/projects/project_c.projects.json")
            )?,
            r#"{
    "projects": [
        {
            "name": "team_zissou/project_c",
            "description": "Stuff relating to project C",
            "targets": [
                "bazel://project_a/...",
                "bazel://project_b/..."
            ],
            "projects": [
                "team_banzai/project_a"
            ]
        }
    ]
}"#
        );
        Ok(())
    }
}
