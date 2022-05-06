mod bazel_de;
mod bazel_resolver;
mod directory_resolver;
mod pants_resolver;

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
    bazel_resolver::BazelResolver, directory_resolver::DirectoryResolver,
    pants_resolver::PantsResolver,
};

/// A request to resolve targets in a particular repository.
#[derive(Clone, Debug)]
pub struct ResolutionRequest {
    pub repo: PathBuf,
    pub coordinate_set: TargetSet,
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
    bazel_resolver: BazelResolver,
    directory_resolver: DirectoryResolver,
    pants_resolver: PantsResolver,
}

impl Resolver for RoutingResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            bazel_resolver: BazelResolver::new(cache_root),
            directory_resolver: DirectoryResolver::new(cache_root),
            pants_resolver: PantsResolver::new(cache_root),
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
            let mut bazel_coordinates = HashSet::new();
            let mut directory_coordinates = HashSet::new();
            let mut pants_coordinates = HashSet::new();
            for target in request.coordinate_set.underlying().iter().cloned() {
                match target {
                    target @ Target::Bazel(_) => {
                        bazel_coordinates.insert(target);
                    }
                    target @ Target::Directory(_) => {
                        directory_coordinates.insert(target);
                    }
                    target @ Target::Pants(_) => {
                        pants_coordinates.insert(target);
                    }
                }
            }

            let bazel_subrequest = ResolutionRequest {
                coordinate_set: bazel_coordinates.into(),
                ..request.clone()
            };
            let directory_subrequest = ResolutionRequest {
                coordinate_set: directory_coordinates.into(),
                ..request.clone()
            };
            let pants_subrequest = ResolutionRequest {
                coordinate_set: pants_coordinates.into(),
                ..request.clone()
            };
            vec![bazel_subrequest, directory_subrequest, pants_subrequest]
        };

        subrequests
            .par_iter()
            .map(|subrequest| {
                let app_clone = app.clone();

                debug_assert!(subrequest.coordinate_set.is_uniform());
                match subrequest.coordinate_set.underlying().iter().next() {
                    Some(Target::Bazel(_)) => {
                        self.bazel_resolver
                            .resolve(subrequest, cache_options, app_clone)
                    }
                    Some(Target::Directory(_)) => {
                        self.directory_resolver
                            .resolve(subrequest, cache_options, app_clone)
                    }
                    Some(Target::Pants(_)) => {
                        self.pants_resolver
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
