use anyhow::{Context, Result};
use focus_util::app::App;
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

/// TODO
pub fn post_merge(_app: Arc<App>) -> Result<()> {
    Ok(())
}

/// TODO
pub fn post_checkout(_app: Arc<App>) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod testing {}
