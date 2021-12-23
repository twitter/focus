mod bazel_resolver;
mod directory_resolver;
mod pants_resolver;

use crate::{
    app::App,
    coordinate::{Coordinate, CoordinateSet},
    util::git_helper::RepoState,
};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use self::{
    bazel_resolver::BazelResolver, directory_resolver::DirectoryResolver,
    pants_resolver::PantsResolver,
};

/// A request to resolve coordinates in a particular repository.
pub struct ResolutionRequest {
    repo: PathBuf,
    repo_state: RepoState,
    coordinate_set: CoordinateSet,
}

impl ResolutionRequest {
    pub fn new(repo: &Path, repo_state: RepoState, coordinate_set: CoordinateSet) -> Self {
        Self {
            repo: repo.to_owned(),
            repo_state,
            coordinate_set,
        }
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    pub fn repo_state(&self) -> &RepoState {
        &self.repo_state
    }

    pub fn coordinate_set(&self) -> &CoordinateSet {
        &self.coordinate_set
    }
}

/// Result of resolving a set of coordinates; namely a set of paths.
#[derive(Default)]
pub struct ResolutionResult {
    paths: BTreeSet<PathBuf>,
}

impl ResolutionResult {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn paths(&self) -> &BTreeSet<PathBuf> {
        &self.paths
    }

    pub fn merge(&mut self, other: &ResolutionResult) {
        self.paths.extend(other.paths().to_owned());
    }
}

impl From<BTreeSet<PathBuf>> for ResolutionResult {
    fn from(paths: BTreeSet<PathBuf>) -> Self {
        Self { paths }
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
            .coordinate_set()
            .underlying()
            .par_iter()
            .map(|coordinate| {
                let app_clone = app.clone();

                let mut set = HashSet::<Coordinate>::new();
                set.insert(coordinate.to_owned());

                let subrequest = ResolutionRequest::new(
                    request.repo(),
                    request.repo_state().clone(),
                    CoordinateSet::from(set),
                );

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
                acc.merge(&result);
                Ok(acc)
            })
            .context("Resolving coordinates failed")
    }
}
