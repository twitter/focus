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
