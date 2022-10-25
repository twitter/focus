// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashSet, path::Path, sync::Arc};

use focus_testing::{init_logging, ScratchGitRepo};

use anyhow::Result;
use focus_util::app::App;
use maplit::hashset;
use tempfile::TempDir;

use super::{selection::Selection, *};

struct Fixture {
    #[allow(dead_code)]
    dir: TempDir,
    underlying: ScratchGitRepo,
    app: Arc<App>,
}

impl Fixture {
    fn new() -> Result<Self> {
        let app = Arc::new(App::new_for_testing()?);
        let dir = TempDir::new()?;
        let path = dir.path().join("dense");
        let branch = String::from("main");
        let repo = ScratchGitRepo::new_copied_fixture(
            app.git_binary().clone(),
            Path::new("bazel_java_example"),
            &path,
            &branch,
        )?;
        Ok(Self {
            dir,
            underlying: repo,
            app,
        })
    }

    fn repo(&self) -> Result<Repo> {
        Repo::open(self.underlying.path(), self.app.clone())
    }
}

/// Extract project names from the selection
fn project_names(selection: &Selection) -> HashSet<String> {
    selection
        .projects
        .iter()
        .map(|project| project.name.clone())
        .collect()
}

#[test]
fn repo_with_no_selections_returns_mandatory_projects_in_computed_selection() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let repo = fixture.repo()?;

    let selection_manager = repo.selection_manager()?;
    let computed_selection = selection_manager.computed_selection()?;
    assert_eq!(
        project_names(&computed_selection),
        hashset! {String::from("mandatory")}
    );
    assert!(computed_selection.targets.is_empty());

    Ok(())
}

#[test]
fn modifying_and_saving_the_selection() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let repo = fixture.repo()?;

    let project_name = String::from("team_banzai/project_a");
    let target = Target::try_from("bazel://library_b/...")?;

    {
        let mut selection_manager = repo.selection_manager()?;
        selection_manager.process(&[
            Operation {
                action: OperationAction::default_add(),
                underlying: Underlying::Project(project_name.clone()),
            },
            Operation {
                action: OperationAction::default_add(),
                underlying: Underlying::Target(target.clone()),
            },
        ])?;
        selection_manager.save()?;
        let computed_selection = selection_manager.computed_selection()?;
        assert_eq!(
            project_names(&computed_selection),
            hashset! {String::from("mandatory"), project_name.clone()}
        );
        assert_eq!(computed_selection.targets, hashset! {target.clone()});
    }

    {
        // Ensure that after loading from disk in a new instance, the selection is the same.
        let mut selection_manager = repo.selection_manager()?;
        let computed_selection = selection_manager.computed_selection()?;
        assert_eq!(
            project_names(&computed_selection),
            hashset! {String::from("mandatory"), project_name.clone()}
        );
        assert_eq!(computed_selection.targets, hashset! {target.clone()});

        // Remove the target
        selection_manager.process(&[Operation {
            action: OperationAction::default_remove(),
            underlying: Underlying::Target(target),
        }])?;
        selection_manager.save()?;
        let computed_selection = selection_manager.computed_selection()?;
        assert_eq!(
            project_names(&computed_selection),
            hashset! {String::from("mandatory"), project_name}
        );
        assert!(computed_selection.targets.is_empty());
    }

    Ok(())
}

#[test]
fn adding_an_unknown_project() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let repo = fixture.repo()?;

    let nonexistent_project = Underlying::Project(String::from("blofeld/moonbase"));
    let mut selection_manager = repo.selection_manager()?;
    let result = selection_manager.process(&[Operation {
        action: OperationAction::default_add(),
        underlying: nonexistent_project.clone(),
    }])?;
    assert!(!result.is_success());
    assert_eq!(result.absent, hashset! {nonexistent_project});

    Ok(())
}

#[test]
fn mandatory_projects_cannot_be_selected() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let repo = fixture.repo()?;

    let mandatory_project = Underlying::Project(String::from("mandatory"));
    let mut selection_manager = repo.selection_manager()?;
    let result = selection_manager.process(&[Operation {
        action: OperationAction::default_add(),
        underlying: mandatory_project.clone(),
    }])?;
    assert!(!result.is_success());
    assert_eq!(result.absent, hashset! {mandatory_project});

    Ok(())
}

#[test]
fn duplicate_projects_are_ignored() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let repo = fixture.repo()?;

    let project_b = Underlying::Project(String::from("team_zissou/project_b"));
    let mut selection_manager = repo.selection_manager()?;
    let result = selection_manager.process(&[Operation {
        action: OperationAction::default_add(),
        underlying: project_b.clone(),
    }])?;
    assert!(result.is_success());
    assert_eq!(result.added, hashset! {project_b.clone()});

    let result = selection_manager.process(&[Operation {
        action: OperationAction::default_add(),
        underlying: project_b.clone(),
    }])?;
    assert!(result.is_success());
    assert_eq!(result.ignored, hashset! {project_b});

    Ok(())
}
