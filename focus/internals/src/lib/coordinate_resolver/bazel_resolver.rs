use std::{
    io::BufRead,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::util::sandbox_command::{SandboxCommand, SandboxCommandOutput};

use anyhow::{bail, Result};

use super::*;

/// Resolves Bazel coordinates to paths
pub struct BazelResolver {
    #[allow(unused)]
    cache_root: PathBuf,

    mutex: Mutex<()>,
}

impl BazelResolver {
    fn locate_bazel_binary(request: &ResolutionRequest) -> PathBuf {
        let in_repo_bazel_wrapper = request.repo().join("bazel");
        if in_repo_bazel_wrapper.is_file() {
            // This is dumb, but our wrapper script pukes if you invoke it with an absolute path. We are just ensuring that it exists at all.
            PathBuf::from("./bazel")
        } else {
            PathBuf::from("bazel")
        }
    }
}

impl Resolver for BazelResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            cache_root: cache_root.join("bazel").to_owned(),
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
        let clauses: Vec<String> = request
            .coordinate_set()
            .underlying()
            .iter()
            .filter_map(|coordinate| {
                // TODO: Consider parameterizing depth
                match coordinate {
                    Coordinate::Bazel(inner) => Some(format!("buildfiles(deps({}))", inner)),
                    _ => unreachable!(),
                }
            })
            .collect();
        let query = clauses.join(" union ");

        // Run Bazel query
        let description = format!("bazel query '{}'", &query);
        let (mut cmd, scmd) = SandboxCommand::new(
            description.clone(),
            Self::locate_bazel_binary(request),
            app.clone(),
        )?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(&query)
                .arg("--output=package")
                .current_dir(request.repo()),
            SandboxCommandOutput::Stderr,
            &description,
        )?;

        let reader = scmd.read_buffered(SandboxCommandOutput::Stdout)?;
        for line in reader.lines() {
            if let Ok(line) = line {
                let path = PathBuf::from(&line);
                if !&line.starts_with("@")
                    && !path.starts_with("bazel-out/")
                    && !path.starts_with("external/")
                {
                    directories.insert(path);
                }
            }
        }
        app.ui().log(String::from("Bazel Resolver"), format!("'{}' requires {} directories", &query, directories.len()));

        Ok(ResolutionResult::from(directories))
    }
}
