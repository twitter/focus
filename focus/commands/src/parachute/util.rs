use std::path::{Path, PathBuf};

pub struct TemporaryWorkingDirectory {
    original_directory: PathBuf,
}

impl TemporaryWorkingDirectory {
    pub(crate) fn new(directory_to_switch_to: &Path) -> anyhow::Result<Self> {
        use std::env;
        use anyhow::Context;
        let current_dir = env::current_dir().context("getting the current directory failed")?;
        env::set_current_dir(directory_to_switch_to).context("switching to the new directory failed")?;
        Ok(Self {
            original_directory: current_dir,
        })
    }
}

impl Drop for TemporaryWorkingDirectory {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original_directory).expect("switching back to the original directory failed");
    }
}
