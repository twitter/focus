use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use tracing::{error, warn};

pub struct LockFile {
    path: PathBuf,
    fd: i32,
}

impl LockFile {
    // Try to obtain an exclusively locked file at the given path. It must not exist.
    pub fn new(path: &Path) -> Result<Self> {
        use std::os::unix::prelude::*;

        if std::fs::metadata(path).is_ok() {
            error!("Another process is holding a lock on {}", path.display());
            error!(
                "The lock is held by {}",
                std::fs::read_to_string(path).context("Failed reading lockfile")?
            );
            bail!("Another process holds a lock");
        }

        match File::create(path) {
            Ok(file) => {
                Self::write_process_description(&file)?;
                let fd = file.into_raw_fd();

                if let Err(e) = Self::acqrel_lock(fd, true) {
                    bail!(
                        "Acquiring exclusive advisory lock on {} failed: {}",
                        path.display(),
                        e
                    );
                }
                Ok(Self {
                    path: path.to_owned(),
                    fd,
                })
            }
            Err(e) => {
                bail!("Creating lock file {} failed: {}", path.display(), e);
            }
        }
    }

    fn write_process_description(file: &File) -> Result<()> {
        let mut buffered_writer = BufWriter::new(file);
        writeln!(
            buffered_writer,
            "PID {} started by {} on host {}",
            std::process::id(),
            whoami::username(),
            whoami::hostname(),
        )?;
        buffered_writer.flush()?;
        file.sync_all()?;

        Ok(())
    }

    fn acqrel_lock(fd: i32, lock: bool) -> Result<()> {
        use nix::*;

        let mut fls: libc::flock = unsafe { core::mem::zeroed() };
        if lock {
            fls.l_type = libc::F_WRLCK as libc::c_short;
        } else {
            fls.l_type = libc::F_UNLCK as libc::c_short;
        }
        fls.l_whence = libc::SEEK_SET as libc::c_short;
        fls.l_start = 0;
        fls.l_len = 0;
        fls.l_pid = unistd::getpid().as_raw();

        if let Err(e) = fcntl::fcntl(fd, fcntl::FcntlArg::F_SETLKW(&fls)) {
            bail!("fnctl error: {}", e);
        }

        Ok(())
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        if let Err(e) = Self::acqrel_lock(self.fd, false) {
            warn!(
                ?self.path,
                ?e,
                "Releasing advisory lock on file failed",
            );
        }
        if let Err(e) = std::fs::remove_file(self.path.as_path()) {
            warn!("Removing lock file {} failed: {}", self.path.display(), e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::cell::Cell;
    use tempfile::tempdir;

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
        LockFile::new(path.as_path())
            .err()
            .expect("should have failed");

        Ok(())
    }
}
