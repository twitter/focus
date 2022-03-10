use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use tracing::warn;

lazy_static! {
    pub static ref MAIN_SEPARATOR_PATH: PathBuf =
        PathBuf::from(format!("{}", std::path::MAIN_SEPARATOR));
}

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

pub(crate) fn find_closest_directory_with_build_file<P0: AsRef<Path>, P1: AsRef<Path>>(
    file: P0,
    ceiling: P1,
) -> Result<Option<PathBuf>> {
    let file = file.as_ref();
    let ceiling = ceiling.as_ref();
    let mut dir = if file.is_dir() {
        file
    } else if let Some(parent) = file.parent() {
        parent
    } else {
        warn!(?file, "Path has no parent");
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
            let file_name = entry.file_name();
            let entry_path = Path::new(&file_name);
            if is_build_definition(entry_path) {
                // Match BUILD, BUILD.*
                return Ok(Some(dir.to_owned()));
            }
        }

        dir = dir
            .parent()
            .context("getting parent of current directory")?;
    }
}

lazy_static! {
    static ref BUILD_STEM: OsString = OsString::from("BUILD");
    static ref WORKSPACE_STEM: OsString = OsString::from("WORKSPACE");
    static ref STARLARK_EXTENSION: OsString = OsString::from("bzl");
}

/// Determine if the Path is a build definition.
pub(crate) fn is_build_definition<P: AsRef<Path>>(path: P) -> bool {
    let path = path.as_ref();
    match path.file_stem() {
        Some(stem) => stem.eq(BUILD_STEM.as_os_str()),
        None => false,
    }
}

/// Determine if the Path is a file relevant to the build graph.
pub(crate) fn is_relevant_to_build_graph<P: AsRef<Path>>(path: P) -> bool {
    let path = path.as_ref();
    if let Some(stem) = path.file_stem() {
        if stem.eq(WORKSPACE_STEM.as_os_str()) || stem.eq(BUILD_STEM.as_os_str()) {
            return true;
        }
    }

    if let Some(extension) = path.extension() {
        return extension.eq(STARLARK_EXTENSION.as_os_str());
    }

    false
}

pub fn expand_tilde<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
    let p = path.as_ref();
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

/// Determine if the `subject` is under `ancestor`.
pub fn has_ancestor<P: AsRef<Path>>(subject: P, ancestor: P) -> Result<bool> {
    let subject = subject.as_ref();
    let ancestor = ancestor.as_ref();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_relevant_to_build_graph() {
        assert!(is_relevant_to_build_graph(&Path::new("WORKSPACE.weird")));
        assert!(is_relevant_to_build_graph(&Path::new("WORKSPACE")));
        assert!(is_relevant_to_build_graph(&Path::new("BUILD.weird")));
        assert!(is_relevant_to_build_graph(&Path::new("BUILD")));
        assert!(is_relevant_to_build_graph(&Path::new("jank.bzl")));
        assert!(!is_relevant_to_build_graph(&Path::new("foo.c")));
    }

    #[test]
    fn test_is_involved_in_build() {
        assert!(is_build_definition(&Path::new("BUILD.funky")));
        assert!(is_build_definition(&Path::new("BUILD")));
        assert!(!is_build_definition(&Path::new("bar.c")));
    }
}
