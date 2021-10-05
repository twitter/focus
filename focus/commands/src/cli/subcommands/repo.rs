use anyhow::{Context, Result};

use crate::tracker::Tracker;

pub fn list() -> Result<()> {
    let tracker = Tracker::default();
    let snapshot = tracker.scan().context("scanning repositories")?;
    for repo in snapshot.repos() {
        println!("{}", repo)
    }

    Ok(())
}
