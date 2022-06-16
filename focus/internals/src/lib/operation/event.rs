use anyhow::{Context, Result};
use focus_util::app::{App, ExitCode};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::{fs::File, path::Path, sync::Arc};
use tracing::debug;

/// Initializes hooks in passed in repo
pub fn init(repo_path: &Path) -> Result<()> {
    debug!("Writing hooks to {}", repo_path.display());

    let hooks = vec!["post-merge", "post-commit"];
    let hooks_dir = repo_path.join(".git").join("hooks");
    write_hooks_to_dir(&hooks, &hooks_dir)?;

    Ok(())
}

fn write_hooks_to_dir(hooks: &[&str], dir: &Path) -> Result<()> {
    for hook in hooks {
        let focus_exe = &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("focus"));
        let focus_exe_path = focus_exe.file_name().unwrap().to_string_lossy();
        let contents = format!("{} event {}", focus_exe_path, hook);

        let file_path = dir.join(hook);
        let mut file = File::options()
            .write(true)
            .create(true)
            .mode(0o755)
            .open(&file_path)
            .context(format!("opening/creating {}", hook))?;
        writeln!(file, "{}", contents)
            .context(format!("writing contents to {}", file_path.display()))?;
    }

    Ok(())
}

pub fn post_merge(app: Arc<App>) -> Result<ExitCode> {
    let current_dir = std::env::current_dir().context("Failed to obtain current directory")?;
    debug!(sparse_repo = ?current_dir.display(), "Running post-merge hook");
    crate::operation::sync::run(&current_dir, false, app)?;
    Ok(ExitCode(0))
}

pub fn post_checkout(_app: Arc<App>) -> Result<ExitCode> {
    let current_dir = std::env::current_dir().context("Failed to obtain current directory")?;
    debug!(sparse_repo = ?current_dir.display(), "Running post-checkout hook");
    Ok(ExitCode(0))
}

pub fn post_commit(_app: Arc<App>) -> Result<ExitCode> {
    let current_dir = std::env::current_dir().context("Failed to obtain current directory")?;
    debug!(sparse_repo = ?current_dir.display(), "Running post-commit hook");
    Ok(ExitCode(0))
}

#[cfg(test)]
mod testing {
    use anyhow::Result;
    use std::fs;

    use super::*;

    #[test]
    fn write_hooks_to_dir_produces_correct_scripts() -> Result<()> {
        let focus_exe = &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("focus"));
        let focus_exe_path = focus_exe.file_name().unwrap().to_string_lossy();
        let hook_names = vec!["fancy-hook", "boring-hook"];
        let temp_dir = tempfile::tempdir()?;
        let temp_dir_path = temp_dir.path();

        write_hooks_to_dir(&hook_names, temp_dir_path)?;

        for hook in hook_names {
            let expected_content = format!("{} event {}\n", focus_exe_path, hook);
            let content = fs::read_to_string(temp_dir_path.join(hook))
                .context(format!("Could not read hook {}", hook))?;
            assert_eq!(content, expected_content);
        }
        Ok(())
    }
}
