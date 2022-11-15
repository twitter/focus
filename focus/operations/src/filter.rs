// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use focus_internals::model::repo::Repo;
use focus_util::app::{App, ExitCode};
use std::{path::Path, sync::Arc};
use tracing::info;

use crate::sync::SyncRequest;

pub fn run(
    sparse_repo: impl AsRef<Path>,
    app: Arc<App>,
    filter_value: bool,
    run_sync: bool,
) -> Result<ExitCode> {
    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let working_tree = repo.working_tree()?;
    let prev_filter_value = working_tree.get_filter_config()?;

    // no change, no need to update the work tree
    if prev_filter_value == filter_value {
        let s = if filter_value {
            "filtered"
        } else {
            "unfiltered"
        };
        info!("No change to configuration. Remaining in {} view.", s);
        return Ok(ExitCode(0));
    }

    // set to the new passed in value
    working_tree.set_filter_config(filter_value)?;

    if !filter_value {
        info!("Turning filter off. Going into unfiltered view.");
        working_tree.switch_filter_off(app)?;
    } else {
        info!("Turning filter on. Going into filtered view.");
        working_tree.switch_filter_on(app.clone())?;
        if run_sync {
            crate::sync::run(
                &SyncRequest::new(sparse_repo.as_ref(), crate::sync::SyncMode::Incremental),
                app,
            )?;
        }
    }

    Ok(ExitCode(0))
}
