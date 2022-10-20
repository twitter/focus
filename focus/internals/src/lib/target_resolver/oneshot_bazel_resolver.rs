// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Mutex,
};

use anyhow::{bail, Result};
use focus_util::sandbox_command::{SandboxCommand, SandboxCommandOutput};
use tracing::{debug, info};

use crate::target::Label;

use super::*;

const OUTLINING_BAZELRC_PATH: &str = "focus/outlining.bazelrc";

/// Resolves Bazel targets to paths
pub struct OneShotBazelResolver {
    #[allow(dead_code)]
    cache_root: PathBuf,

    mutex: Mutex<()>,
}

impl OneShotBazelResolver {
    fn locate_bazel_binary(request: &ResolutionRequest) -> PathBuf {
        let in_repo_bazel_wrapper = request.repo.join("bazel");
        if in_repo_bazel_wrapper.is_file() {
            // This is dumb, but our wrapper script pukes if you invoke it with an absolute path. We are just ensuring that it exists at all.
            PathBuf::from("./bazel")
        } else {
            PathBuf::from("bazel")
        }
    }
}

impl Resolver for OneShotBazelResolver {
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

        let mut paths = BTreeSet::new();
        let package_deps = BTreeMap::new();
        #[allow(clippy::redundant_clone)]
        let app = app.clone();

        let labels: HashSet<&Label> = request
            .targets
            .iter()
            .filter_map(|target| {
                // TODO: Consider parameterizing depth
                match target {
                    Target::Bazel(label) => Some(label),
                    _ => unreachable!(),
                }
            })
            .collect();

        // let paths = {
        let query = format!(
            // Use `deps(...)` so that we preserve the actual names of the
            // targets which were declared as dependencies, but also add in
            // any `buildfiles` dependencies that might exist in the
            // repository. This includes dependencies on `BUILD` or `.bzl`
            // files (such as those `load`ed by the other `BUILD` files).
            //
            // We limit buildfile dependencies to only those in the
            // repository, because `.bzl` files, etc., in external
            // repositories are typically not supported, so querying them
            // fails.
            "deps({0})",
            bazel_common::make_set(labels.iter().copied())
        );

        let result = Self::run_bazel_package_query(app.clone(), request, &query)?;
        for line in result {
            paths.insert(PathBuf::from_str(line.as_str())?);
        }

        info!("'{}' requires {} packages", &query, paths.len());

        Ok(ResolutionResult {
            paths,
            package_deps,
        })
    }
}

impl OneShotBazelResolver {
    fn run_bazel_package_query(
        app: Arc<App>,
        request: &ResolutionRequest,
        query: &str,
    ) -> Result<Vec<String>> {
        let query_file_path = {
            let (mut file, path, _serial) = app
                .sandbox()
                .create_file(Some("bazel_query"), None, None)
                .context("creating bazel query file")?;
            file.write_all(query.as_bytes())
                .context("writing bazel query to disk")?;
            path
        };

        let mut initial_bazel_args = Vec::<String>::new();
        if request.repo.join(OUTLINING_BAZELRC_PATH).is_file() {
            initial_bazel_args.push(String::from("--noworkspace_rc"));
            initial_bazel_args.push(format!("--bazelrc={}", OUTLINING_BAZELRC_PATH));
        }
        let (mut cmd, scmd) = SandboxCommand::new(Self::locate_bazel_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.args(initial_bazel_args)
                .arg("query")
                .arg("--output=package")
                .arg("--order_output=no")
                .arg("--noimplicit_deps")
                .arg("--nofetch")
                .arg("--experimental_repository_disable_download=true")
                .arg("--query_file")
                .arg(query_file_path)
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
        )?;

        // Read to string so that we can print it if we need to debug.
        let raw_result = {
            let mut result = String::new();
            scmd.read_to_string(SandboxCommandOutput::Stdout, &mut result)?;
            result
        };

        debug!(?query, ?raw_result, "Query returned with result");
        Ok(raw_result.lines().map(|s| s.to_owned()).collect())
    }
}
