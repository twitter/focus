// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{path::Path, sync::Arc};

//use anyhow::{Context, Result};
use anyhow::Result;
use focus_internals::model::repo::Repo;
use focus_util::app::{App, ExitCode};

pub fn lint(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<ExitCode> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    for (_, project) in selections
        .project_catalog()
        .optional_projects
        .underlying
        .clone()
        .into_iter()
    {
        project.lint()?;
    }
    println!("Pass");
    Ok(ExitCode(0))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use focus_internals::model::repo::Repo;
    use focus_testing::ScratchGitRepo;
    use focus_util::app::{App, ExitCode};

    use crate::project::lint;
    #[test]
    pub fn test_lint() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;
        fix.write_and_commit_file(
            "focus/projects/good.projects.json",
            r#"
{
    "projects": [
        {
            "name": "a/good/project",
            "description": "a good project",
            "targets": ["bazel://a_good_project"],
            "projects": ["some/other/project"]
        }
    ]
}
        "#,
            "adding a good project",
        )?;
        let testing_app = Arc::new(App::new_for_testing()?);
        let lint_result = lint(fix.path(), testing_app.clone());
        assert_eq!(lint_result?, ExitCode(0));
        let repo = Repo::open(fix.path(), testing_app)?;
        let optional_projects = repo
            .selection_manager()?
            .project_catalog()
            .optional_projects
            .underlying
            .clone();
        assert!(optional_projects.contains_key("a/good/project"));
        Ok(())
    }

    #[test]
    pub fn test_lint_bad_project() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;
        fix.write_and_commit_file(
            "focus/projects/bad.projects.json",
            r#"
{
    "projects": [
        {
            "name": "a bad project",
        }
    ]
}
        "#,
            "adding a bad project",
        )?;
        let testing_app = Arc::new(App::new_for_testing()?);
        let lint_result = lint(fix.path(), testing_app);
        assert!(lint_result.is_err());
        Ok(())
    }
}
