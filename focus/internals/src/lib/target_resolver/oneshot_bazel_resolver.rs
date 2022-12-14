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

        let query = format!(
            "deps({0}) union kind(rule, filter('^//', buildfiles(deps({0}))))",
            bazel_common::make_set(labels.iter().copied())
        );

        let result = Self::run_bazel_package_query(app, request, &query)?;
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
        scmd.ensure_exit_with_status_or_log(
            cmd.args(initial_bazel_args)
                .arg("query")
                .arg("--output=package")
                .arg("--order_output=no")
                .arg("--noimplicit_deps")
                .arg("--query_file")
                .arg(query_file_path)
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
            &[0, 3],
            // TODO: Attempt to disable fetching to speed Bazel further.
            //   --nofetch --experimental_repository_disable_download=true --keep_going
            //
            // We will have to allow Bazel PARTIAL_ANALYSIS_FAILURE because of --nofetch.
            //
            // See https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/util/ExitCode.java.
        )?;

        // Read to string so that we can print it if we need to debug.
        let raw_result = {
            let mut result = String::new();
            scmd.read_to_string(SandboxCommandOutput::Stdout, &mut result)?;
            result
        };

        debug!(?query, ?raw_result, "Query returned with result");
        Ok(raw_result
            .lines()
            .filter_map(|s| {
                if !s.starts_with('@') {
                    Some(s.to_owned())
                } else {
                    None
                }
            })
            .collect())
    }
}
