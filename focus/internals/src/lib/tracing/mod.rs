pub mod chrome;
pub mod focus;
pub mod git_trace2;

use std::path::{Path, PathBuf};

pub use chrome::Trace;
pub use focus::{init_tracing, Guard, TracingOpts};

use anyhow::Result;

#[cfg(test)]
pub(in crate::tracing) mod testing {
    use std::path::{Path, PathBuf};

    use anyhow::{anyhow, Result};

    pub const FIXTURE_RELPATH: &str = "src/lib/tracing/fixtures";

    pub fn manifest_dir() -> Result<PathBuf> {
        Ok(PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR")
                .ok_or_else(|| anyhow!("CARGO_MANIFEST_DIR not set"))?,
        ))
    }

    pub fn manifest_relative_path<S: AsRef<Path>>(s: S) -> Result<PathBuf> {
        Ok(manifest_dir()?.join(s.as_ref()))
    }

    pub fn fixture_path<S: AsRef<Path>>(s: S) -> Result<PathBuf> {
        manifest_relative_path(FIXTURE_RELPATH).map(|p| p.join(s.as_ref()))
    }
}

fn home_relative_path<P: AsRef<Path>>(p: P) -> Result<PathBuf> {
    match dirs::home_dir().map(|pb| pb.join(p.as_ref())) {
        Some(path) => Ok(path),
        None => Err(anyhow::anyhow!("HOME not defined")),
    }
}

#[cfg(target_os = "macos")]
const DEFAULT_LOG_DIR: &str = "Library/Logs/focus";

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_arch = "wasm32"
)))]
const DEFAULT_LOG_DIR: &str = ".local/focus/log";

/// returns the default system specific log location
pub fn log_dir() -> Result<PathBuf> {
    home_relative_path(DEFAULT_LOG_DIR)
}
