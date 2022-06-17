use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::os::unix::prelude::RawFd;
use std::path::{Path, PathBuf};
use tracing::{error, warn};

pub struct LockFile {
    path: PathBuf,
    fd: i32,
}

impl LockFile {
    // Try to obtain an exclusively locked file at the given path
    pub fn new(path: &Path) -> Result<Self> {
        use std::os::unix::prelude::*;

        let res = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path);

        match res {
            Ok(mut file) => {
                // acquire the exclusive lock
                if let Err(e) = Self::acqrel_lock(file.as_raw_fd(), true) {
                    error!("Another process is holding a lock on {}", path.display());
                    error!(
                        "The lock is held by {}",
                        std::fs::read_to_string(path).context("Failed reading lockfile")?
                    );
                    bail!(
                        "Acquiring exclusive advisory lock on {} failed: {}",
                        path.display(),
                        e
                    );
                }

                // if we acquired the lock, write our process info to the lock
                Self::write_process_description(&mut file)?;

                Ok(Self {
                    path: path.to_owned(),
                    fd: file.into_raw_fd(),
                })
            }
            Err(e) => {
                bail!("Creating lock file {} failed: {:?}", path.display(), e);
            }
        }
    }

    // we must own the exclusive lock before writing the file!
    fn write_process_description(file: &mut File) -> Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        {
            let fp: File = file.try_clone()?;
            let mut buffered_writer = BufWriter::new(fp);
            writeln!(
                buffered_writer,
                "{}",
                super::process::get_process_description()
            )?;
            buffered_writer.flush()?;
        }

        file.sync_all()?;

        Ok(())
    }

    fn acqrel_lock(fd: RawFd, lock: bool) -> Result<()> {
        use nix::*;

        let op = if lock {
            libc::LOCK_EX | libc::LOCK_NB
        } else {
            libc::LOCK_UN
        };

        let ret = unsafe { libc::flock(fd, op) };
        if ret < 0 {
            bail!(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(self.path.as_path()) {
            warn!(?self.path, ?e, "Removing lock file failed");
        }
        if let Err(e) = Self::acqrel_lock(self.fd, false) {
            warn!(
                ?self.path,
                ?e,
                "Releasing advisory lock on file failed",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use focus_testing as testing;

    use super::*;
    use anyhow::Result;
    use std::cell::Cell;
    use tempfile::tempdir;

    #[test]
    fn creating_a_lock() -> Result<()> {
        testing::init_logging();
        let dir = tempdir()?;
        let path = dir.path().join("lockfile");

        LockFile::new(&path).expect("should have succeeded");

        Ok(())
    }

    #[test]
    fn failing_to_create_a_duplicate_lock() -> Result<()> {
        testing::init_logging();
        let dir = tempdir()?;
        let path = dir.path().join("lockfile");

        let _a = LockFile::new(&path).expect("should have succeeded");
        let _b = LockFile::new(&path).err().expect("should have failed");

        Ok(())
    }

    #[test]
    fn lock_should_be_cleaned_up_after_drop() -> Result<()> {
        testing::init_logging();
        let dir = tempdir()?;
        let path = dir.path().join("lockfile");

        {
            let _a = LockFile::new(&path).expect("should have acquired lock");
        }

        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn lock_should_contain_process_info() -> Result<()> {
        testing::init_logging();
        let dir = tempdir()?;
        let path = dir.path().join("lockfile");

        let _a = LockFile::new(&path).expect("should have acquired lock");

        let content = std::fs::read_to_string(&path).context("Failed reading lockfile")?;

        let content = content.trim();

        let expect = format!(
            "PID {} started by {} on host {}",
            std::process::id(),
            whoami::username(),
            whoami::hostname(),
        );

        assert_eq!(expect.as_str(), content);

        Ok(())
    }

    #[test]
    fn failing_to_create_a_lock_in_an_inextant_directory() -> Result<()> {
        testing::init_logging();
        let path: Cell<Option<PathBuf>> = Cell::new(None);
        {
            let dir = tempdir()?;
            let inner_path = dir.path().join("lockfile");
            path.replace(Some(inner_path));
        }

        let path = path.take().unwrap();
        LockFile::new(path.as_path())
            .err()
            .expect("should have failed");

        Ok(())
    }
}
