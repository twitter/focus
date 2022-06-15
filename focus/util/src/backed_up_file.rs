use anyhow::{Context, Result};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
};
use tracing::{debug, info};

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

        debug!(?path, ?backup_path, "Backed up file",);

        Ok(Self {
            original_path: path.to_owned(),
            backup_path: backup_path.to_owned(),
            restore: Cell::new(true),
        })
    }

    /// Sets whether the backup should be restored.
    pub fn set_restore(&self, new_value: bool) {
        self.restore.set(new_value);
    }

    /// Prevent the backup from being restored.
    pub fn discard(&self) {
        self.set_restore(false);
    }
}

impl Drop for BackedUpFile {
    fn drop(&mut self) {
        if self.restore.get() {
            info!(
                ?self.backup_path,
                ?self.original_path,
                "Restoring backed up file",
            );
            std::fs::rename(&self.backup_path, &self.original_path)
                .expect("failed to restore backup file");
        } else {
            debug!(?self.backup_path, "Removing backup file");
            std::fs::remove_file(&self.backup_path).expect("failed to delete backup file");
        }
    }
}
