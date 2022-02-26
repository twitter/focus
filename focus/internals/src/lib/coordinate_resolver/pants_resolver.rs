use std::{
    io::BufRead,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{bail, Result};

use crate::util::sandbox_command::{SandboxCommand, SandboxCommandOutput};

use super::*;

/// Resolves Pants coordinates to paths using the `filedeps` goal.
pub struct PantsResolver {
    #[allow(dead_code)]
    cache_root: PathBuf,

    mutex: Mutex<()>,
}

impl PantsResolver {
    fn locate_pants_binary(request: &ResolutionRequest) -> PathBuf {
        let in_repo_pants_wrapper = request.repo.join("pants");
        if in_repo_pants_wrapper.is_file() {
            // This is dumb, but our wrapper script pukes if you invoke it with an absolute path. We are just ensuring that it exists at all.
            PathBuf::from("./pants")
        } else {
            PathBuf::from("pants")
        }
    }
}

impl Resolver for PantsResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            cache_root: cache_root.join("bazel"),
            mutex: Mutex::new(()),
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        _cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult> {
        let lock = self.mutex.lock();
        if let Err(e) = lock {
            bail!("Failed to lock mutex: {}", e);
        }

        let mut directories = BTreeSet::<PathBuf>::new();
        let addresses: Vec<String> = request
            .coordinate_set
            .underlying()
            .iter()
            .filter_map(|coordinate| {
                // TODO: Consider parameterizing depth
                match coordinate {
                    Coordinate::Pants(inner) => Some(inner.to_owned()),
                    _ => unreachable!(),
                }
            })
            .collect();

        // Run Pants query
        let mut args = vec![String::from("filedeps")];
        args.extend(addresses);
        let args_description = args.join(" ");

        let description = format!("pants {}", &args_description);
        let (mut cmd, scmd) =
            SandboxCommand::new(description.clone(), Self::locate_pants_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.env("EE_PANTS_DAEMON_BETA", "0")
                .args(args)
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
            &description,
        )?;

        let reader = scmd.read_buffered(SandboxCommandOutput::Stdout)?;
        #[allow(clippy::manual_flatten)]
        for line in reader.lines() {
            if let Ok(line) = line {
                let path = PathBuf::from(&line);
                // Let's drop the filename
                if let Some(parent) = path.parent() {
                    directories.insert(parent.to_owned());
                } else {
                    log::error!("Could not get parent of path {}", path.display());
                    // Complain...
                }
            }
        }

        Ok(ResolutionResult::from(directories))
    }
}
