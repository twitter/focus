// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use anyhow::Result;
use focus_internals::model::repo::Repo;

fn migrations() -> Migrations {
    vec![
        Box::new(HooksMigration),
        Box::new(UseOneshotSyncByDefaultMigration),
    ]
}

fn runner_for_repo(repo_path: &Path, app: Arc<App>) -> Result<Runner> {
    let focus_dir = repo_path.join(".focus").join("manifest.json");
    Runner::new(repo_path, &focus_dir, migrations(), app)
}

pub fn is_upgrade_required(repo_path: &Path, app: Arc<App>) -> Result<bool> {
    runner_for_repo(repo_path, app).and_then(|runner| runner.is_upgrade_required())
}

pub fn perform_pending_migrations(repo_path: &Path, app: Arc<App>) -> Result<bool> {
    runner_for_repo(repo_path, app).and_then(|runner| runner.perform_pending_migrations())
}

struct HooksMigration;
impl Migration for HooksMigration {
    fn id(&self) -> Identifier {
        Identifier::Serial(1)
    }

    fn description(&self) -> &str {
        "Initialize the repo with required hooks"
    }

    fn upgrade(&self, path: &Path, _app: Arc<App>) -> Result<()> {
        focus_operations::event::init(path)
    }
}

struct UseOneshotSyncByDefaultMigration;
impl Migration for UseOneshotSyncByDefaultMigration {
    fn id(&self) -> Identifier {
        Identifier::Serial(2)
    }

    fn description(&self) -> &str {
        "Make one-shot sync the default"
    }

    fn upgrade(&self, path: &Path, app: Arc<App>) -> Result<()> {
        let repo = Repo::open(path, app)?;
        repo.set_bazel_oneshot_resolution(true)?;
        Ok(())
    }
}
