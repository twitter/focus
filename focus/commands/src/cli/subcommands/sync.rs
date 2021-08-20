use std::path::Path;

use anyhow::{bail, Context, Error, Result};

use crate::{sandbox::Sandbox, working_tree_synchronizer::WorkingTreeSynchronizer};

pub fn perform<F, J>(description: &str, f: F) -> Result<J>
where
    F: FnOnce() -> Result<J>,
{
    log::debug!("Task started: {}", description);
    let result = f();
    if let Err(e) = result {
        log::error!("Task failed: {}: {}", description, e);
        bail!(e);
    }
    log::info!("Task succeded: {}", description);

    result
}

pub fn run(sandbox: &Sandbox, dense_repo: &Path, sparse_repo: &Path) -> Result<()> {
    let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, &sandbox)?;
    let sparse_sync = WorkingTreeSynchronizer::new(&sparse_repo, &sandbox)?;

    if let Ok(clean) = perform("Checking that sparse repo is in a clean state", || {
        sparse_sync.is_working_tree_clean()
    }) {
        if !clean {
            bail!("The sparse repo is not clean");
        }
    }

    if let Ok(clean) = perform("Checking that dense repo is in a clean state", || {
        dense_sync.is_working_tree_clean()
    }) {
        if !clean {
            bail!("The dense repo is not clean");
        }
    }

    let sparse_commit = perform("Getting the sparse repo branch name", || {
        sparse_sync.read_head()
    })?;

    let sparse_branch = perform("Getting the sparse repo branch name", || {
        sparse_sync.read_branch()
    })?;

    perform("Push from the sparse repo to the dense repo", || {
        sparse_sync.push_to_remote("dense", &sparse_branch)
    })?;

    // perform("Check out the current sparse repo commit in the dense repo", || {
    //     let commit_id = String::from_utf8(sparse_commit)?;
    //     dense_sync.checkout_orphaned(&commit_id)
    // })?;

    perform("Determine the directories for the sparse checkout in the dense repo", || {
        // Use the sparse commit here
        todo!("implement this");

    })?;

    Ok(())
}
