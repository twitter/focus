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
    use nix::fcntl::FlockArg;
    use nix::unistd::getpid;

    pub(crate) struct LockFile {
        path: PathBuf,
        fd: i32,
    }

    impl LockFile {
        // Try to obtain an exclusively locked file at the given path. It must not exist.
        pub fn new(path: &Path) -> Result<Self> {
            use std::fs::File;
            use std::os::unix::prelude::*;

            if std::fs::metadata(path).is_ok() {
                bail!("File {} already exists", path.display());
            }

            match File::create(path) {
                Ok(mut file) => {
                    let fd = file.into_raw_fd();

                    if let Err(e) = Self::acqrel_lock(fd, true) {
                        bail!("Acquiring exclusive advisory lock on {} failed: {}", path.display(), e);
                    }
                    Ok(Self { path: path.to_owned(), fd })
                },
                Err(e) => {
                    bail!("Creating lock file {} failed: {}", path.display(), e);
                }
            }
        }

            fn acqrel_lock(fd: i32, lock: bool) -> Result<()> {
                use nix::*;

                let mut fls: libc::flock = unsafe {
                    core::mem::zeroed()
                };
                if lock {
                    fls.l_type = libc::F_WRLCK as libc::c_short;
                } else {
                    fls.l_type = libc::F_UNLCK as libc::c_short;
                }
                fls.l_whence = libc::SEEK_SET as libc::c_short;
                fls.l_start = 0;
                fls.l_len = 0;
                fls.l_pid = getpid().as_raw();

                if let Err(e) = fcntl::fcntl(fd, fcntl::FcntlArg::F_SETLKW(&fls)) {
                    bail!("fnctl error: {}", e);
                }

                Ok(())
            }
        }

        impl Drop for LockFile {
            fn drop(&mut self) {
                if let Err(e) = Self::acqrel_lock(self.fd, false) {
                    warn!("Releasing advisory lock on file {} failed: {}", self.path.display(), e);
                }
                if let Err(e) = std::fs::remove_file(self.path.as_path()) {
                    warn!("Removing lock file {} failed: {}", self.path.display(), e);
                }
            }
        }

        #[cfg(test)]
        mod tests {
            use anyhow::Result;
            use super::*;
            use tempfile::tempdir;
            use std::cell::Cell;

            #[test]
            fn creating_a_lock() -> Result<()> {
                let dir = tempdir()?;
                let path = dir.path().join("lockfile");

                LockFile::new(&path).expect("should have succeeded");

                Ok(())
            }

            #[test]
            fn failing_to_create_a_duplicate_lock() -> Result<()> {
                let dir = tempdir()?;
                let path = dir.path().join("lockfile");

                let _a = LockFile::new(&path).expect("should have succeeded");
                let _b = LockFile::new(&path).err().expect("should have failed");

                Ok(())
            }

            #[test]
            fn failing_to_create_a_lock_in_an_inextant_directory() -> Result<()> {
                let path: Cell<Option<PathBuf>> = Cell::new(None);
                {
                    let dir = tempdir()?;
                    let inner_path = dir.path().join("lockfile");
                    path.replace(Some(inner_path));
                }

                let path = path.take().unwrap();
                LockFile::new(path.as_path()).err().expect("should have failed");

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
