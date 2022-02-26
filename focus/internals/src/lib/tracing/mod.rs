pub mod chrome;
pub mod git_trace2;

pub use chrome::Trace;

#[cfg(test)]
pub(in crate::tracing) mod testing {
    use std::path::{Path, PathBuf};

    use anyhow::{anyhow, Result};

    pub const FIXTURE_RELPATH: &str = "src/lib/tracing/fixtures";

    pub fn manifest_dir() -> Result<PathBuf> {
        Ok(PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").ok_or(anyhow!("CARGO_MANIFEST_DIR not set"))?,
        ))
    }

    pub fn manifest_relative_path<S: AsRef<Path>>(s: S) -> Result<PathBuf> {
        Ok(manifest_dir()?.join(s.as_ref()))
    }

    pub fn fixture_path<S: AsRef<Path>>(s: S) -> Result<PathBuf> {
        manifest_relative_path(FIXTURE_RELPATH).map(|p| p.join(s.as_ref()))
    }
}
