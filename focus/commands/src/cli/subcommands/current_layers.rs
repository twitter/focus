use std::path::Path;

use anyhow::{Context, Result};

pub fn run(sparse_repo: &Path) -> Result<()> {
    println!("List current layers in {}", sparse_repo.display());
    Ok(())
}
