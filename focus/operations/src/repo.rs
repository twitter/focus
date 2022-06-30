// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use anyhow::{Context, Result};

use focus_internals::tracker::Tracker;
use focus_util::app::App;

pub fn list(tracker: &Tracker) -> Result<()> {
    let snapshot = tracker.scan().context("scanning repositories")?;
    for repo in snapshot.repos() {
        println!("{}", repo)
    }

    Ok(())
}

pub fn repair(tracker: &Tracker, app: Arc<App>) -> Result<()> {
    tracker
        .repair(app)
        .context("Failed to repair repository registry")
}
