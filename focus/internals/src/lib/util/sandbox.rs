use anyhow::{Context, Result};
use log;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use tempfile::TempDir;

pub struct Sandbox {
    #[allow(dead_code)]
    temp_dir: Option<tempfile::TempDir>,
    path: PathBuf,
    serial_sequence: AtomicUsize,
}

impl Sandbox {
    pub fn new(preserve_contents: bool) -> Result<Self> {
        let underlying: TempDir = tempfile::Builder::new()
            .prefix("focus.")
            .tempdir()
            .context("creating a temporary directory")?;
        let path: PathBuf = (&underlying.path().to_path_buf()).to_owned();

        let temp_dir: Option<TempDir> = if preserve_contents {
            // We preserve the contents of the temporary directory by dropping and recreating it.
            drop(underlying);

            fs::create_dir_all(&path).context("recreating the directory")?;
            log::info!(
                "Created sandbox in '{}', which will not be cleaned up at exit",
                &path.display()
            );

            None
        } else {
            Some(underlying)
        };

        let serial_sequence = AtomicUsize::new(0);

        Ok(Self {
            temp_dir,
            path,
            serial_sequence,
        })
    }

    pub fn create_file(
        &self,
        prefix: Option<&str>,
        extension: Option<&str>,
        serial: Option<usize>,
    ) -> Result<(File, PathBuf, usize)> {
        let parent = self.path.to_owned();
        let mut path = PathBuf::new();
        let serial: usize =
            serial.unwrap_or_else(|| self.serial_sequence.fetch_add(1, Ordering::SeqCst));
        let name = format!("{}-{:09}", prefix.unwrap_or("unknown"), serial);
        path.set_file_name(name);
        if let Some(extension) = extension {
            path.set_extension(extension);
        }
        let qualified_path = parent.join(path);
        let file = File::create(&qualified_path.as_path()).context("creating a temporary file")?;

        Ok((file, qualified_path, serial))
    }

    pub fn create_subdirectory(&self, prefix: &str) -> Result<PathBuf> {
        let parent = self.path.to_owned();
        let serial: usize = self.serial_sequence.fetch_add(1, Ordering::SeqCst);
        let name = format!("{}-{:09}", prefix, serial);
        let qualified_path = parent.join(name);
        std::fs::create_dir(qualified_path.as_path())
            .context("creating sandbox subdirectory failed")?;
        Ok(qualified_path)
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl Clone for Sandbox {
    fn clone(&self) -> Self {
        let serial: usize = self.serial_sequence.fetch_add(1, Ordering::SeqCst);
        let label = format!("subsandbox-{}", serial);
        let path = self.path.join(label);
        if let Err(_e) = std::fs::create_dir(path.as_path()) {
            panic!(
                "creating directory for cloned sandbox ({}) failed",
                &path.display()
            );
        }

        Self {
            temp_dir: None,
            path,
            serial_sequence: AtomicUsize::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{bail, Result};
    use std::ffi::OsStr;
    use std::fs;

    #[test]
    fn sandbox_deletion() -> Result<()> {
        let path = {
            let sandbox = Sandbox::new(false)?;
            let owned_path = sandbox.path().to_owned();
            owned_path
        };
        assert!(fs::metadata(path).is_err());
        Ok(())
    }

    #[test]
    fn sandbox_preservation() -> Result<()> {
        let sandbox = Sandbox::new(true)?;
        let path = sandbox.path().to_owned();
        drop(sandbox);
        assert!(fs::metadata(&path)?.is_dir());
        fs::remove_dir(&path)?;
        Ok(())
    }

    #[test]
    fn file_naming() -> Result<()> {
        let sandbox = Sandbox::new(true)?;
        match sandbox.create_file(Some("hello"), Some("txt"), None) {
            Ok((_, path, ser)) => {
                assert_eq!(ser, 0);
                let s = format!("hello-{:09}.txt", ser);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            }
            _ => bail!("expected a file"),
        }
        match sandbox.create_file(None, Some("txt"), None) {
            Ok((_, path, ser)) => {
                assert_eq!(ser, 1);
                let s = format!("unknown-{:09}.txt", ser);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            }
            _ => bail!("expected a file"),
        }
        match sandbox.create_file(Some("adieu"), None, None) {
            Ok((_, path, ser)) => {
                assert_eq!(ser, 2);
                let s = format!("adieu-{:09}", ser);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            }
            _ => bail!("expected a file"),
        }
        match sandbox.create_file(Some("adieu"), Some("too"), Some(2 as usize)) {
            Ok((_, path, ser)) => {
                assert_eq!(ser, 2);
                let s = format!("adieu-{:09}.too", ser);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            }
            _ => bail!("expected a file"),
        }

        Ok(())
    }
}
