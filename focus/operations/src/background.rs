// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use focus_internals::model::repo::Repo;
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};

use focus_util::app::{App, ExitCode};

use crate::sync::SyncMode;

pub fn enable(
    app: Arc<App>,
    sparse_repo: PathBuf,
    idle_period_ms: u64,
) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo, app)?;
    repo.set_preemptive_sync_enabled(true)?;
    repo.set_preemptive_sync_idle_threshold(Duration::from_millis(idle_period_ms))?;

    Ok(ExitCode(0))
}

pub fn disable(app: Arc<App>, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo, app)?;
    repo.set_preemptive_sync_enabled(false)?;

    Ok(ExitCode(0))
}

pub fn sync(app: Arc<App>, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    super::sync::run(&sparse_repo, SyncMode::Preemptive { force: true }, app)
        .context("Running preemptive sync")?;
    Ok(ExitCode(0))
}
