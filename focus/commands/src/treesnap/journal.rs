use anyhow::{bail, Error, Result};

use internals::error::AppError;
use internals::util::lock_file::LockFile;
use log::warn;
use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempDir};
use walkdir::DirEntry;

lazy_static! {
    static ref JOURNAL_INDEX_EXTENSION: OsString = OsString::from("journal_index");
    static ref JOURNAL_DATA_EXTENSION: OsString = OsString::from("journal_data");
    static ref JOURNAL_DATA_MAX_SIZE: u32 = u32::max_value(); // TODO: Fix deprecated function
}

// Contains diagnostic error messages pertaining to a journal index or data file at the indicated path
pub(crate) struct JournalError {
    path: PathBuf,
    kind: JournalFileKind,
    diagnostics: Vec<String>,
}

enum JournalFileKind {
    Index = 0,
    Data = 1,
}

struct JournalManager {
    dir: PathBuf,
}

impl JournalManager {
    pub fn new(dir: &Path) -> Result<Self> {
        Ok(Self {
            dir: dir.to_owned(),
        })
    }

    fn determine_file_kind(path: &Path) -> Option<JournalFileKind> {
        match path.extension() {
            Some(extension) => {
                if extension == JOURNAL_INDEX_EXTENSION.as_os_str() {
                    Some(JournalFileKind::Index)
                } else if extension == JOURNAL_DATA_EXTENSION.as_os_str() {
                    Some(JournalFileKind::Data)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    // Verify that all of the journals in the directory are consistent with the indices
    fn verify_one(path: &Path) -> Result<bool> {
        todo!("impl")
    }

    fn verify_all(&self) -> Result<Vec<JournalError>> {
        todo!("impl")
    }

    fn lock_file_path(&self) -> PathBuf {
        self.dir.join("LOCK")
    }

    pub fn try_locking_directory(&self) -> Result<LockFile> {
        LockFile::new(self.lock_file_path().as_path())
    }

    // Returns a pair of vectors, the first containing paths of the index files, the second
    // containing the journal data files.
    pub fn locate_index_and_data_files(&self) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut index_files = Vec::<PathBuf>::new();
        let mut data_files = Vec::<PathBuf>::new();

        for entry in walkdir::WalkDir::new(&self.dir).max_depth(1) {
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    match Self::determine_file_kind(&path) {
                        Some(kind) => match kind {
                            JournalFileKind::Index => index_files.push(path.to_owned()),
                            JournalFileKind::Data => data_files.push(path.to_owned()),
                        },
                        None => {
                            // Ignore unrelated file
                        }
                    }
                }
                Err(e) => bail!("Enumerating files failed: {}", e),
            }
        }

        Ok((index_files, data_files))
    }

    pub fn count() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use tempfile::tempdir;

    #[test]
    fn journal_manager_new() -> Result<()> {
        let temp_dir = tempdir()?;
        let temp_dir_path = temp_dir.path().to_owned();
        let manager = JournalManager::new(&temp_dir_path.as_path());

        Ok(())
    }
}
