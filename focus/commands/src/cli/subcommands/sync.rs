use std::{fmt::format, path::Path};

use anyhow::{bail, Context, Error, Result};

use crate::{sandbox::Sandbox, working_tree_synchronizer::WorkingTreeSynchronizer};

pub fn perform<F, J>(description: &str, f: F) -> Result<J>
where
    F: FnOnce() -> Result<J>,
{
    eprint!("{} ... ", description);
    let result = f();
    if let Err(e) = result {
        eprintln!("FAILED!");
        bail!("Task {} failed: {}", description, e);
    }
    eprintln!("OK");
    log::info!("Task {} succeeded", description);

    result
}

pub fn run(sandbox: &Sandbox, dense_repo: &Path, sparse_repo: &Path) -> Result<()> {
    let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, &sandbox)?;

    if let Ok(clean) = perform("Checking that dense repo is in a clean state", || {
        dense_sync.is_working_tree_clean()
    }) {
        if !clean {
            bail!("The dense repo is not clean");
        }
    }

    // let sparse_sync = WorkingTreeSynchronizer::new(&sparse_repo, &sandbox)?;
    // Check that the dense repo is a clean state.
    // Check that the dense repo is at the same ref as the sparse repo
    // Otherwise push to it

    // Apply the layer set to the sparse profile
    Ok(())
}
