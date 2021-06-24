use anyhow::{Result, Error, bail};

use std::path::{PathBuf, Path};
use walkdir::DirEntry;
use std::ffi::{OsStr, OsString};
use tempfile::{TempDir, NamedTempFile};
use std::fs::{File, OpenOptions};
use std::cell::Cell;
use internals::error::AppError;
use crate::journal::lockfile::LockFile;

lazy_static! {
    static ref JOURNAL_INDEX_EXTENSION: OsString = OsString::from("journal_index");
    static ref JOURNAL_DATA_EXTENSION: OsString = OsString::from("journal_data");
    static ref JOURNAL_DATA_MAX_SIZE: u32 = u32::max_value(); // TODO: Fix deprecated function
}

mod lockfile {
    use anyhow::{bail, Result};
    use std::path::{PathBuf, Path};
    use std::cell::Cell;
    use std::fs::File;
    use log::warn;

    pub(crate) struct LockFile {
        path: PathBuf,
        file: Cell<Option<File>>,
    }

    impl LockFile {
        // Try to obtain an exclusively locked file at the given path. It must not exist.
        pub fn new(path: &Path) -> Result<Self> {
            match File::create(path) {
                Ok(file) => {
                    let file = Cell::new(Some(file));
                    Ok(Self{path: path.to_owned(), file})
                },
                Err(e) => {
                    bail!("Creating exclusive lockfile at path {} failed: {}", &path.display(), e)
                }
            }
        }
    }

    impl Drop for LockFile {
        fn drop(&mut self) {
            self.file.replace(None);
            if let Err(e) = std::fs::remove_file(self.path.as_path()) {
                warn!("Failed to remove lock file '{}': {}", self.path.display(), e);
            }
        }
    }

    #[cfg(test)]
    mod tests{
        use anyhow::Result;
        use super::*;
        use tempfile::tempdir;
        use std::cell::Cell;

        #[test]
        fn creating_a_lock() -> Result<()> {
            let dir = tempdir()?;
            let path = dir.path();

            assert!(LockFile::new(path).is_ok());

            Ok(())
        }

        #[test]
        fn failing_to_create_a_duplicate_lock() -> Result<()> {
            let dir = tempdir()?;
            let path = dir.path();

            assert!(LockFile::new(path).is_ok());
            assert!(LockFile::new(path).is_err());

            Ok(())
        }

        #[test]
        fn failing_to_create_a_lock_in_an_inextant_directory() -> Result<()> {
            let path: Cell<Option<PathBuf>> = Cell::new(None);
            {
                let dir = tempdir()?;
                path.replace(Some(dir.path().to_owned()));
            }

            let path = path.take().unwrap();
            assert!(LockFile::new(path.as_path()).is_err());

            Ok(())

        }
    }

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
                        Some(kind) => {
                            match kind {
                                JournalFileKind::Index => index_files.push(path.to_owned()),
                                JournalFileKind::Data => data_files.push(path.to_owned()),
                            }
                        }
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
    use anyhow::Result;
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn journal_manager_new() -> Result<()> {
        let temp_dir = tempdir()?;
        let temp_dir_path = temp_dir.path().to_owned();
        let manager = JournalManager::new(&temp_dir_path.as_path());

        Ok(())
    }
}
