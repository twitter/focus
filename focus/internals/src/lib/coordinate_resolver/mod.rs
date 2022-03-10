mod bazel_de;
mod bazel_resolver;
mod directory_resolver;
mod pants_resolver;

use crate::{
    app::App,
    coordinate::{Coordinate, CoordinateSet},
    index::{DependencyKey, DependencyValue},
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

/// A request to resolve coordinates in a particular repository.
#[derive(Clone, Debug)]
pub struct ResolutionRequest {
    pub repo: PathBuf,
    pub coordinate_set: CoordinateSet,
}

/// Result of resolving a set of coordinates; namely a set of paths.
#[derive(Debug, Default)]
pub struct ResolutionResult {
    /// The set of files/directories which must be materialized.
    pub paths: BTreeSet<PathBuf>,

    /// The set of coordinates which were resolved as part of this request and
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

        request
            .coordinate_set
            .underlying()
            .par_iter()
            .map(|coordinate| {
                let app_clone = app.clone();

                let mut set = HashSet::<Coordinate>::new();
                set.insert(coordinate.to_owned());

                let subrequest = ResolutionRequest {
                    coordinate_set: CoordinateSet::from(set),
                    ..request.clone()
                };

                match coordinate {
                    Coordinate::Bazel(_) => {
                        self.bazel_resolver
                            .resolve(&subrequest, cache_options, app_clone)
                    }
                    Coordinate::Directory(_) => {
                        self.directory_resolver
                            .resolve(&subrequest, cache_options, app_clone)
                    }
                    Coordinate::Pants(_) => {
                        self.pants_resolver
                            .resolve(&subrequest, cache_options, app_clone)
                    }
                }
                .with_context(|| format!("failed to resolve coordinate {}", coordinate))
            })
            .try_reduce(ResolutionResult::new, |mut acc, result| {
                acc.merge(result);
                Ok(acc)
            })
            .context("Resolving coordinates failed")
    }
}
