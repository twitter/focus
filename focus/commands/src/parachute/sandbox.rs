use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::{Builder, TempDir};
use log;
use std::fs;
use std::fs::File; use std::sync::atomic::{AtomicUsize, Ordering};

struct Sandbox {
    temp_dir: Option<tempfile::TempDir>,
    path: PathBuf,
    serial_sequence: AtomicUsize,
}

impl Sandbox {
    fn new(preserve_contents: bool) -> Result<Self> {
        let underlying: TempDir = tempfile::Builder::new()
            .prefix("focus")
            .tempdir()
            .context("creating a temporary directory")?;
        let path: PathBuf = (&underlying.path().to_path_buf()).to_owned();

        let temp_dir: Option<TempDir> = if preserve_contents {
            // We preserve the contents of the temporary directory by dropping and recreating it.
            drop(&underlying);
            underlying.close().context("closing temporary directory")?;
            fs::create_dir_all(&path).context("recreating the directory")?;
            log::info!("Created sandbox in '{}', which will not be cleaned up at exit", &path.display());

            None
        } else {
            Some(underlying)
        };

        let serial_sequence = AtomicUsize::new(0);

        Ok(Self{temp_dir, path, serial_sequence})
    }

    fn create_file(&self, prefix: Option<&str>, extension: Option<&str>) -> Result<(File, PathBuf)> {
        let mut path = self.path.to_owned();

        let serial: usize = self.serial_sequence.fetch_add(1, Ordering::SeqCst);
        let name = format!("{}-{:x}", prefix.unwrap_or("unknown"), serial);
        path.set_file_name(name);
        if let Some(extension) = extension {
            path.set_extension(extension);
        }

        let mut file = File::create(&path.as_path()).context("creating a temporary file")?;

        Ok((file, path))
    }

    fn path(&self) -> &Path {
        self.path.as_path()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{bail, Result};
    use super::*;
    use std::fs;
    use std::ffi::OsStr;

    #[test]
    fn sandbox_deletion() -> Result<()> {
        let path = {
            let sandbox = Sandbox::new(false)?;
            let owned_path = sandbox.path().to_owned();
            // drop(&sandbox);
            owned_path
        };
        assert!(fs::metadata(path).is_err());
        Ok(())
    }

    #[test]
    fn sandbox_preservation() -> Result<()> {
        let sandbox = Sandbox::new(true)?;
        let path = &sandbox.path().to_owned();
        drop(&sandbox);
        assert!(fs::metadata(path)?.is_dir());
        fs::remove_dir(path);
        Ok(())
    }

    #[test]
    fn file_naming() -> Result<()> {
        let sandbox = Sandbox::new(true)?;
        match sandbox.create_file(Some("hello"), Some("txt")) {
            Ok((_, path)) => {
                let s = format!("hello-{:x}.txt", 0 as usize);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            },
            _ => bail!("expected a file"),
        }
        match sandbox.create_file(None, Some("txt")) {
            Ok((_, path)) => {
                let s = format!("unknown-{:x}.txt", 1 as usize);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            },
            _ => bail!("expected a file"),
        }
        match sandbox.create_file(Some("adieu"), None) {
            Ok((_, path)) => {
                let s = format!("adieu-{:x}", 2 as usize);
                let expected = OsStr::new(&s);
                assert_eq!(&path.file_name().unwrap(), &expected);
            },
            _ => bail!("expected a file"),
        }

        Ok(())
    }
}
