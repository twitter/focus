use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use std::path::{Path, PathBuf};
use tracing::warn;

pub fn assert_focused_repo(path: &Path) -> Result<()> {
    if !path.is_dir() || !path.join(".focus").is_dir() {
        bail!("This does not appear to be a focused repo -- it is missing a `.focus` directory");
    }

    Ok(())
}

pub fn focus_config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("could not determine config dir")
        .join("focus")
}

pub(crate) fn find_closest_directory_with_build_file(
    file: &Path,
    ceiling: &Path,
) -> Result<Option<PathBuf>> {
    let mut dir = if file.is_dir() {
        file
    } else if let Some(parent) = file.parent() {
        parent
    } else {
        warn!("Path {} has no parent", file.display());
        return Ok(None);
    };
    loop {
        if dir == ceiling {
            return Ok(None);
        }

        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("reading directory contents {}", dir.display()))?
        {
            let entry = entry.context("reading directory entry")?;
            if entry.file_name() == "BUILD" {
                // Match BUILD, BUILD.*
                return Ok(Some(dir.to_owned()));
            }
        }

        dir = dir
            .parent()
            .context("getting parent of current directory")?;
    }
}

pub fn expand_tilde<P: AsRef<Path>>(path_user_input: P) -> Result<PathBuf> {
    let p = path_user_input.as_ref();
    if !p.starts_with("~") {
        return Ok(p.to_path_buf());
    }
    if p == Path::new("~") {
        if let Some(home_dir) = dirs::home_dir() {
            return Ok(home_dir);
        } else {
            bail!("Could not determine home directory");
        }
    }

    let result = dirs::home_dir().map(|mut h| {
        if h == Path::new("/") {
            // Corner case: `h` root directory;
            // don't prepend extra `/`, just drop the tilde.
            p.strip_prefix("~").unwrap().to_path_buf()
        } else {
            h.push(p.strip_prefix("~/").unwrap());
            h
        }
    });

    if let Some(path) = result {
        Ok(path)
    } else {
        bail!("Failed to expand tildes in path '{}'", p.display());
    }
}

pub fn has_ancestor(subject: &Path, ancestor: &Path) -> Result<bool> {
    if subject == ancestor {
        return Ok(true);
    }

    let mut subject = subject;
    while let Some(parent) = subject.parent() {
        if parent == ancestor {
            return Ok(true);
        }

        subject = parent;
    }

    Ok(false)
}
lazy_static! {
    pub static ref SLASH_PATH: PathBuf = PathBuf::from("/");
}
