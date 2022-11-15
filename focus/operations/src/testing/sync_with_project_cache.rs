// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use anyhow::{bail, Context, Result};
use focus_internals::model::repo::PROJECT_CACHE_ENDPOINT_CONFIG_KEY;
use focus_testing::init_logging;
use focus_util::{app::ExitCode, git_helper};

use crate::{
    project_cache,
    sync::{SyncMechanism, SyncRequest, SYNC_FROM_PROJECT_CACHE_REQUIRED_ERROR_MESSAGE},
    testing::integration::RepoPairFixture,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Location {
    Sparse,
    Dense,
}

struct Fixture {
    underlying: RepoPairFixture,
    location: Location,
}

impl Fixture {
    fn new(location: Location) -> Result<Self> {
        let underlying = RepoPairFixture::new()?;
        underlying.perform_clone()?;
        Ok(Self {
            underlying,
            location,
        })
    }

    fn repo_path(&self) -> &Path {
        self.repo_path_for_location(self.location)
    }

    fn repo_path_for_location(&self, location: Location) -> &Path {
        match location {
            Location::Sparse => self.underlying.sparse_repo_path.as_path(),
            Location::Dense => self.underlying.dense_repo_path.as_path(),
        }
    }

    fn configure_endpoint(&self, location: Location) -> Result<()> {
        let path = self.underlying.dir.path().join("project_cache_db");
        std::fs::create_dir_all(&path)?;
        git_helper::write_config(
            self.repo_path_for_location(location),
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
                self.repo_path(),
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

    let fixture = Fixture::new(Location::Sparse)?;
    fixture.configure_endpoint(Location::Sparse)?;
    fixture.generate_content(2)?;

    Ok(())
}

#[test]
fn project_cache_falls_back_with_non_project_targets_selected() -> Result<()> {
    init_logging();

    let fixture = Fixture::new(Location::Sparse)?;
    let app = fixture.underlying.app.clone();
    fixture.configure_endpoint(Location::Sparse)?;

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
        &SyncRequest::new(
            &fixture.underlying.sparse_repo_path,
            crate::sync::SyncMode::RequireProjectCache,
        ),
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

    let fixture = Fixture::new(Location::Sparse)?;
    let app = fixture.underlying.app.clone();
    fixture.configure_endpoint(Location::Sparse)?;
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
        &SyncRequest::new(
            &fixture.underlying.sparse_repo_path,
            crate::sync::SyncMode::RequireProjectCache,
        ),
        app,
    )?;
    assert_eq!(result.mechanism, SyncMechanism::ProjectCache);

    Ok(())
}

fn project_cache_generates_all_projects_internal(location: Location) -> Result<()> {
    init_logging();

    let fixture = Fixture::new(location)?;
    let app = fixture.underlying.app.clone();
    if location == Location::Dense {
        // Also configure sparse since we will query there.
        fixture.configure_endpoint(Location::Sparse)?;
    }
    fixture.configure_endpoint(location)?;
    fixture.generate_content(10)?;

    tracing::debug!(repo = ?fixture.repo_path());

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
        &SyncRequest::new(
            &fixture.underlying.sparse_repo_path,
            crate::sync::SyncMode::RequireProjectCache,
        ),
        app,
    )?;
    assert_eq!(result.mechanism, SyncMechanism::ProjectCache);

    Ok(())
}

#[test]
fn project_cache_generates_all_projects_with_sparse_repo() -> Result<()> {
    project_cache_generates_all_projects_internal(Location::Sparse)
}

// #[ignore = "Needs underlying implementation"]
#[test]
fn project_cache_generates_all_projects_with_dense_repo() -> Result<()> {
    project_cache_generates_all_projects_internal(Location::Dense)
}
