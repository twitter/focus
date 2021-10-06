use std::sync::Arc;

use anyhow::{Context, Result};

use crate::{app::App, tracker::Tracker};

pub fn list() -> Result<()> {
    let tracker = Tracker::default();
    let snapshot = tracker.scan().context("scanning repositories")?;
    for repo in snapshot.repos() {
        println!("{}", repo)
    }

    Ok(())
}

pub fn repair(app: Arc<App>) -> Result<()> {
    Tracker::default().repair(app).context("Failed to repair repository registry")
}