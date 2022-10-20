// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

mod bazel_common;
mod bazel_de;
mod directory_resolver;
mod incremental_bazel_resolver;
mod oneshot_bazel_resolver;

use focus_util::app::App;

use crate::{
    index::{DependencyKey, DependencyValue},
    target::{Target, TargetSet},
};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) use self::{
    directory_resolver::DirectoryResolver, incremental_bazel_resolver::IncrementalBazelResolver,
    oneshot_bazel_resolver::OneShotBazelResolver,
};

/// Directs the strategy to resolve Bazel targets.
#[derive(Clone, Debug)]
pub enum BazelResolutionStrategy {
    /// Resolve targets incrementally using content caching.
    Incremental,

    /// Resolve targets in one shot without caching.
    OneShot,
}

/// A set of options guiding resolution.
#[derive(Clone, Debug)]
pub struct ResolutionOptions {
    pub(crate) bazel_resolution_strategy: BazelResolutionStrategy,
}

impl Default for ResolutionOptions {
    fn default() -> Self {
        Self {
            bazel_resolution_strategy: BazelResolutionStrategy::Incremental,
        }
    }
}

/// A request to resolve targets in a particular repository.
#[derive(Clone, Debug, Default)]
pub struct ResolutionRequest {
    pub repo: PathBuf,
    pub targets: TargetSet,
    pub options: ResolutionOptions,
}

/// Result of resolving a set of targets; namely a set of paths.
#[derive(Debug, Default)]
pub struct ResolutionResult {
    /// The set of files/directories which must be materialized.
    pub paths: BTreeSet<PathBuf>,

    /// The set of targets which were resolved as part of this request and
    /// the dependencies they had.
    pub package_deps: BTreeMap<DependencyKey, DependencyValue>,
}

impl ResolutionResult {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn merge(&mut self, other: ResolutionResult) {
        let Self {
            paths,
            package_deps,
        } = other;
        self.paths.extend(paths);
        self.package_deps.extend(package_deps);
    }
}

impl From<BTreeSet<PathBuf>> for ResolutionResult {
    fn from(paths: BTreeSet<PathBuf>) -> Self {
        Self {
            paths,
            package_deps: Default::default(),
        }
    }
}

/// Dictates whether the resolver can retrieve or store responses to a cache.
pub struct CacheOptions {
    #[allow(dead_code)]
    accept_cached_response: bool,
    #[allow(dead_code)]
    store_response_in_cache: bool,
}

impl CacheOptions {
    pub fn new(accept_cached_response: bool, store_response_in_cache: bool) -> Self {
        Self {
            accept_cached_response,
            store_response_in_cache,
        }
    }
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            accept_cached_response: true,
            store_response_in_cache: true,
        }
    }
}

pub trait Resolver {
    fn new(cache_root: &Path) -> Self;

    fn resolve(
        &self,
        request: &ResolutionRequest,
        cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult>;
}

pub struct RoutingResolver {
    incremental_bazel_resolver: IncrementalBazelResolver,
    oneshot_bazel_resolver: OneShotBazelResolver,
    directory_resolver: DirectoryResolver,
}

impl Resolver for RoutingResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            incremental_bazel_resolver: IncrementalBazelResolver::new(cache_root),
            oneshot_bazel_resolver: OneShotBazelResolver::new(cache_root),
            directory_resolver: DirectoryResolver::new(cache_root),
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult> {
        use rayon::prelude::*;

        let subrequests = {
            let mut bazel_targets = HashSet::new();
            let mut directory_targets = HashSet::new();
            for target in request.targets.iter().cloned() {
                match target {
                    target @ Target::Bazel(_) => {
                        bazel_targets.insert(target);
                    }
                    target @ Target::Directory(_) => {
                        directory_targets.insert(target);
                    }
                }
            }

            let bazel_subrequest = ResolutionRequest {
                targets: bazel_targets,
                ..request.clone()
            };
            let directory_subrequest = ResolutionRequest {
                targets: directory_targets,
                ..request.clone()
            };
            vec![bazel_subrequest, directory_subrequest]
        };

        subrequests
            .par_iter()
            .map(|subrequest| {
                let app_clone = app.clone();

                match subrequest.targets.iter().next() {
                    Some(Target::Bazel(_)) => {
                        match subrequest.options.bazel_resolution_strategy {
                            BazelResolutionStrategy::Incremental => self
                                .incremental_bazel_resolver
                                .resolve(subrequest, cache_options, app_clone),
                            BazelResolutionStrategy::OneShot => self
                                .oneshot_bazel_resolver
                                .resolve(subrequest, cache_options, app_clone),
                        }
                    }
                    Some(Target::Directory(_)) => {
                        self.directory_resolver
                            .resolve(subrequest, cache_options, app_clone)
                    }
                    None => Ok(Default::default()),
                }
            })
            .try_reduce(ResolutionResult::new, |mut acc, result| {
                acc.merge(result);
                Ok(acc)
            })
            .context("Resolving targets failed")
    }
}
