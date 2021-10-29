use anyhow::{bail, Result};
use std::path::Path;

pub fn assert_focused_repo(path: &Path) -> Result<()> {
    if !path.is_dir() || !path.join(".focus").is_dir() {
        bail!("This does not appear to be a focused repo -- it is missing a `.focus` directory");
    }

    Ok(())
}
