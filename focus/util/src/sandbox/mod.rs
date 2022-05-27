pub mod cleanup;

use anyhow::{bail, Context, Result};
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::info;

use tempfile::TempDir;

use crate::paths;

pub struct Sandbox {
    #[allow(dead_code)]
    temp_dir: Option<tempfile::TempDir>,
    path: PathBuf,
    serial_sequence: AtomicUsize,
}

const DEFAULT_NAME_PREFIX: &str = "focus_sandbox_";

impl Sandbox {
    pub fn new(
        description: Option<&str>,
        preserve_contents: bool,
        name_prefix: Option<&str>,
    ) -> Result<Self> {
        use std::io::Write;

        let sandbox_root = paths::focus_sandbox_dir();
        std::fs::create_dir_all(&sandbox_root)
            .with_context(|| format!("creating sandbox root {}", sandbox_root.display()))?;

        let underlying: TempDir = tempfile::Builder::new()
            .prefix(&match name_prefix {
                Some(prefix) => DEFAULT_NAME_PREFIX.to_string() + prefix,
                None => DEFAULT_NAME_PREFIX.to_string(),
            })
            .tempdir_in(sandbox_root)
            .context("creating a temporary directory to house the sandbox")?;

        let path: PathBuf = (&underlying.path().to_path_buf()).to_owned();

        let temp_dir: Option<TempDir> = if preserve_contents {
            // We preserve the contents of the temporary directory by dropping and recreating it.
            drop(underlying);

            fs::create_dir_all(&path).context("recreating the directory")?;
            info!(
                ?path,
                "Created sandbox, which will not be cleaned up at exit",
            );

            None
        } else {
            Some(underlying)
        };

        let serial_sequence = AtomicUsize::new(0);
        let instance = Self {
            temp_dir,
            path,
            serial_sequence,
        };

        if let Some(description) = description {
            let description_path = instance.command_description_path();
            match File::create(&description_path) {
                Ok(mut f) => {
                    writeln!(f, "{}", description)?;
                }
                Err(e) => {
                    bail!(
                        "failed writing description file to {}: {}",
                        description_path.display(),
                        e
                    );
                }
            }
        }

        Ok(instance)
    }

    pub fn command_description_path(&self) -> PathBuf {
        self.path.join("cmd")
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
            let sandbox = Sandbox::new(None, false, None)?;
            let owned_path = sandbox.path().to_owned();
            owned_path
        };
        assert!(fs::metadata(path).is_err());
        Ok(())
    }

    #[test]
    fn sandbox_preservation() -> Result<()> {
        let sandbox = Sandbox::new(None, true, None)?;
        let path = sandbox.path().to_owned();
        drop(sandbox);
        assert!(fs::metadata(&path)?.is_dir());
        fs::remove_dir(&path)?;
        Ok(())
    }

    #[test]
    fn sandbox_name_prefix_is_present() -> Result<()> {
        let unnamed_sandbox = Sandbox::new(None, false, None)?;
        assert!(unnamed_sandbox
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&DEFAULT_NAME_PREFIX));

        let named_sandbox = Sandbox::new(None, false, Some("test_"))?;
        assert!(named_sandbox
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&DEFAULT_NAME_PREFIX));

        Ok(())
    }

    #[test]
    fn file_naming() -> Result<()> {
        let sandbox = Sandbox::new(None, true, None)?;
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
        match sandbox.create_file(Some("adieu"), Some("too"), Some(2_usize)) {
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

    #[test]
    fn writing_command_descripton() -> Result<()> {
        let description = "hello";
        let sandbox = Sandbox::new(Some(description), true, None)?;
        assert_eq!(
            fs::read_to_string(sandbox.command_description_path())?,
            format!("{}\n", description)
        );
        Ok(())
    }
}
