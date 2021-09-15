use std::{path::Path, time::SystemTime};

use anyhow::{bail, Context, Result};

use crate::sandbox::Sandbox;

fn unix_epoch_timestamp() -> Result<u64> {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(n) => Ok(n.as_secs()),
        Err(_) => bail!("SystemTime before UNIX EPOCH!"),
    }
}

pub fn run(_sandbox: &Sandbox, _dense_repo: &Path, sparse_repo: &Path) -> Result<()> {
    let invalidation_file_path = sparse_repo.join(".focus").join("invalidated");
    std::fs::write(
        invalidation_file_path.as_path(),
        format!(
            "{}",
            unix_epoch_timestamp().context("obtaining unix timestamp")?
        ),
    )
    .with_context(|| {
        format!(
            "writing invalidation file to {}",
            invalidation_file_path.display()
        )
    })?;

    Ok(())
}
