use anyhow::{Context, Result};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct BackedUpFile {
    original_path: PathBuf,
    backup_path: PathBuf,
    restore: Cell<bool>,
}

impl BackedUpFile {
    pub fn new(path: &Path) -> Result<Self> {
        let mut backup_path = path.to_owned();
        let mut name = backup_path
            .file_name()
            .expect("Backup file with no file name")
            .to_owned();
        name.push(".backup");
        backup_path.set_file_name(name);

        std::fs::copy(&path, &backup_path).with_context(|| {
            format!(
                "Copying {} to the backup file {}",
                &path.display(),
                &backup_path.display()
            )
        })?;

        log::debug!(
            "Backed up {} -> {}",
            &path.display(),
            &backup_path.display()
        );

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
        if self.restore.get() {
            log::info!(
                "Restoring backup {} -> {}",
                self.backup_path.display(),
                self.original_path.display()
            );
            std::fs::rename(&self.backup_path, &self.original_path)
                .expect("failed to restore backup file");
        } else {
            log::debug!("Removing backup {}", self.backup_path.display());
            std::fs::remove_file(&self.backup_path).expect("failed to delete backup file");
        }
    }
}
