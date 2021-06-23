use anyhow::{Result, Error, bail};

use std::path::{PathBuf, Path};
use walkdir::DirEntry;
use std::ffi::{OsStr, OsString};
use tempfile::{TempDir, NamedTempFile};
use std::fs::{File, OpenOptions};
use std::cell::Cell;

lazy_static! {
    static ref JOURNAL_INDEX_EXTENSION: OsString = OsString::from("journal_index");
    static ref JOURNAL_DATA_EXTENSION: OsString = OsString::from("journal_data");
    static ref JOURNAL_DATA_MAX_SIZE: u32 = u32::max_value(); // TODO: Fix deprecated function
}

pub(crate) struct LockFile {
    path: PathBuf,
    file: Cell<File>,
}

impl LockFile {
    // Try to obtain an exclusively locked file at the given path. It must not exist.
    fn try_create(path: &Path) -> Result<Self> {
        match File::create(path) {
            Ok(file) => {
                Ok(Self{path: path.to_owned(), file: Cell::new(file)})
            },
            Err(e) => {
                bail!("Creating exclusive lockfile at path {} failed: {}", &path.display(), e)
            }
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {

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

    pub fn try_lock_directory(&self) -> Result<bool> {
        todo!("impl")
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

    // pub fn append(entry: journal_proto::)

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
