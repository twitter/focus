use anyhow::{Context, Result};
use std::{cell::Cell, path::{Path, PathBuf}};

pub struct BackedUpFile {
    original_path: PathBuf,
    backup_path: PathBuf,
    restore: Cell<bool>,
}

impl BackedUpFile {
    pub fn new(path: &Path) -> Result<Self> {
        let backup_path = path.join(".backup");
        std::fs::copy(&path, &backup_path).context("copying to the backup")?;
        Ok(Self {
            original_path: path.to_owned(),
            backup_path: backup_path.to_owned(),
            restore: Cell::new(true),
        })
    }

    pub fn set_restore(&self, new_value: bool) {
        self.restore.set(new_value);
    }
}

impl Drop for BackedUpFile {
    fn drop(&mut self) {
        if !self.restore.get() {
            std::fs::copy(&self.backup_path, &self.original_path)
            .expect("failed to restore backup file");
        }
        std::fs::remove_file(&self.backup_path).expect("failed to delete backup file");
    }
}
