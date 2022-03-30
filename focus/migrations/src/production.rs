use super::*;
use anyhow::Result;

fn migrations() -> Migrations {
    vec![]
}

fn runner_for_repo(repo_path: &Path) -> Result<Runner> {
    let focus_dir = repo_path.join(".focus").join("manifest.json");
    Runner::new(repo_path, &focus_dir, migrations())
}

pub fn is_upgrade_required(repo_path: &Path) -> Result<bool> {
    runner_for_repo(repo_path).and_then(|runner| runner.is_upgrade_required())
}

pub fn perform_pending_migrations(repo_path: &Path) -> Result<bool> {
    runner_for_repo(repo_path).and_then(|runner| runner.perform_pending_migrations())
}
