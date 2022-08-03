// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use focus_internals::tracker::Tracker;

use anyhow::Result;

use focus_testing::init_logging;

use crate::testing::integration::RepoPairFixture;

#[test]
fn repo_register_after_move() -> Result<()> {
    init_logging();

    let tracker = Tracker::for_testing()?;
    tracker.ensure_directories_exist()?;

    {
        let snapshot = tracker.scan()?;
        assert!(snapshot.repos().is_empty());
    }

    let fixture = RepoPairFixture::new()?;
    fixture.perform_clone()?;
    tracker.ensure_registered(&fixture.sparse_repo_path, fixture.app.clone())?;

    let repo = fixture.sparse_repo()?;
    let working_tree = repo.working_tree().unwrap();
    let id = { working_tree.read_uuid()?.unwrap() };

    {
        let snapshot = tracker.scan()?;
        tracing::debug!(repos = ?snapshot.repos(), "Repos");
        assert!(snapshot.find_repo_by_id(id.as_bytes()).is_some());
    }

    let new_path = fixture.sparse_repo_path.with_extension("moved");

    std::fs::rename(fixture.sparse_repo_path.as_path(), new_path.as_path())?;

    {
        let snapshot = tracker.scan()?;
        assert!(snapshot.find_repo_by_id(id.as_bytes()).is_none());
    }

    crate::repo::register(new_path.as_path(), &tracker, fixture.app.clone())?;

    {
        let snapshot = tracker.scan()?;
        assert!(snapshot.find_repo_by_id(id.as_bytes()).is_some());
    }

    Ok(())
}
