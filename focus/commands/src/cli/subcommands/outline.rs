use anyhow::{bail, Context, Result};


pub fn run(source: &Path, mount_point: &Path, coordinates: Vec<String>) -> Result<()> {
    let source = std::fs::canonicalize(source).context("canonicalizing source path")?;
    let mount_point = std::fs::canonicalize(mount_point).context("canonicalizing mount point path")?;

    Ok(())
}
