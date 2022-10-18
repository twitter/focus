// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use focus_internals::model::repo::PROJECT_CACHE_ENDPOINT_CONFIG_KEY;
use focus_testing::init_logging;
use focus_util::{app::ExitCode, git_helper};

use crate::{
    project_cache,
    sync::{SyncMechanism, SYNC_FROM_PROJECT_CACHE_REQUIRED_ERROR_MESSAGE},
    testing::integration::RepoPairFixture,
};

struct Fixture {
    underlying: RepoPairFixture,
}

impl Fixture {
    fn new() -> Result<Self> {
        let underlying = RepoPairFixture::new()?;
        underlying.perform_clone()?;
        Ok(Self { underlying })
    }

    fn configure_endpoint(&self) -> Result<()> {
        let path = self.underlying.dir.path().join("project_cache_db");
        std::fs::create_dir_all(&path)?;
        git_helper::write_config(
            &self.underlying.sparse_repo_path,
            PROJECT_CACHE_ENDPOINT_CONFIG_KEY,
            format!("file://{}", path.display()).as_str(),
            self.underlying.app.clone(),
        )?;
        Ok(())
    }

    fn generate_content(&self, shard_count: usize) -> Result<()> {
        let app = self.underlying.app.clone();

        for shard_index in 0..shard_count {
            let exit_code = project_cache::push(
                app.clone(),
                &self.underlying.sparse_repo_path,
                String::from("HEAD"),
                shard_index,
                shard_count,
            )
            .with_context(|| {
                format!(
                    "Failed to generate content for shard {} of {}",
                    shard_index + 1,
                    shard_count
                )
            })?;
            if exit_code != ExitCode(0) {
                bail!(
                    "Non-zero exit while generating content for shard {} of {}",
                    shard_index + 1,
                    shard_count
                )
            }
        }

        Ok(())
    }
}

#[test]
fn generation() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    fixture.configure_endpoint()?;
    fixture.generate_content(2)?;

    Ok(())
}

#[test]
fn project_cache_falls_back_with_non_project_targets_selected() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let app = fixture.underlying.app.clone();
    fixture.configure_endpoint()?;

    // Add a project and a directory target to the selection
    crate::selection::add(
        &fixture.underlying.sparse_repo_path,
        false,
        vec![
            String::from("team_banzai/project_a"),
            String::from("directory:w_dir"),
        ],
        false,
        app.clone(),
    )?;

    // Verify that syncing with the project cache fails
    match crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        crate::sync::SyncMode::RequireProjectCache,
        app,
    ) {
        Err(e) => {
            let error_string = e.to_string();
            assert!(error_string.contains(SYNC_FROM_PROJECT_CACHE_REQUIRED_ERROR_MESSAGE))
        }
        Ok(_) => bail!("Expected an error!"),
    }

    Ok(())
}

#[test]
fn project_cache_answers_with_only_projects_selected() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let app = fixture.underlying.app.clone();
    fixture.configure_endpoint()?;
    fixture.generate_content(2)?;

    // Add a project and a directory target to the selection
    crate::selection::add(
        &fixture.underlying.sparse_repo_path,
        true,
        vec![String::from("team_banzai/project_a")],
        false,
        app.clone(),
    )?;

    // Verify that syncing with the project cache fails
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        crate::sync::SyncMode::RequireProjectCache,
        app,
    )?;
    assert_eq!(result.mechanism, SyncMechanism::ProjectCache);

    Ok(())
}

#[test]
fn project_cache_generates_all_projects() -> Result<()> {
    init_logging();

    let fixture = Fixture::new()?;
    let app = fixture.underlying.app.clone();
    fixture.configure_endpoint()?;
    fixture.generate_content(10)?;

    let selection_manager = fixture.underlying.sparse_repo()?.selection_manager()?;
    let project_names: Vec<String> = selection_manager
        .project_catalog()
        .optional_projects
        .underlying
        .iter()
        .map(|(name, _)| name.to_owned())
        .collect();
    assert!(!project_names.is_empty());

    crate::selection::add(
        &fixture.underlying.sparse_repo_path,
        true,
        project_names,
        false,
        app.clone(),
    )?;

    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        crate::sync::SyncMode::RequireProjectCache,
        app,
    )?;
    assert_eq!(result.mechanism, SyncMechanism::ProjectCache);

    Ok(())
}
