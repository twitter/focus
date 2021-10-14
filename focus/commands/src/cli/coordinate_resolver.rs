use crate::{
    app::App,
    coordinate::{Coordinate, CoordinateSet},
    git_helper::RepoState,
    util::sandbox_command::{SandboxCommand, SandboxCommandOutput},
};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeSet, HashSet},
    io::BufRead,
    iter::FromIterator,
    path::{Path, PathBuf},
    sync::Arc,
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

impl Default for ResolutionResult {
    fn default() -> Self {
        Self {
            paths: Default::default(),
        }
    }
}

/// Dictates whether the resolver can retrieve or store responses to a cache.
pub struct CacheOptions {
    #[allow(unused)]
    accept_cached_response: bool,
    #[allow(unused)]
    store_response_in_cache: bool,
}

impl CacheOptions {
    #[allow(unused)]
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
}

impl Resolver for RoutingResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            bazel_resolver: BazelResolver::new(cache_root),
            directory_resolver: DirectoryResolver::new(cache_root),
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult> {
        let coordinate_set = request.coordinate_set().underlying();

        let mut aggregated_result = ResolutionResult::new();
        for (_index, coordinate) in coordinate_set.iter().enumerate() {
            let app_clone = app.clone();

            let mut set = HashSet::<Coordinate>::new();
            set.insert(coordinate.to_owned());

            let subrequest = ResolutionRequest::new(
                request.repo(),
                request.repo_state().clone(),
                CoordinateSet::from(set),
            );

            let result = match coordinate {
                Coordinate::Bazel(_) => {
                    self.bazel_resolver
                        .resolve(&subrequest, &cache_options, app_clone)
                }
                Coordinate::Directory(_) => {
                    self.directory_resolver
                        .resolve(&subrequest, &cache_options, app_clone)
                }
            }
            .with_context(|| format!("failed to resolve coordinate {}", coordinate))?;
            aggregated_result.merge(&result);
        }

        Ok(aggregated_result)
    }
}

/// Resolves Bazel coordinates to paths
struct BazelResolver {
    #[allow(unused)]
    cache_root: PathBuf,
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
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        _cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult> {
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
        let (mut cmd, scmd) =
            SandboxCommand::new(description, Self::locate_bazel_binary(request), app.clone())?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(query)
                .arg("--output=package")
                .current_dir(request.repo()),
            SandboxCommandOutput::Stderr,
            "bazel query",
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

        Ok(ResolutionResult::from(directories))
    }
}

/// Resolves directories verbatim
struct DirectoryResolver {
    #[allow(unused)]
    cache_root: PathBuf,
}

impl Resolver for DirectoryResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            cache_root: cache_root.join("directory").to_owned(),
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        _cache_options: &CacheOptions,
        _app: Arc<App>,
    ) -> Result<ResolutionResult> {
        let directories = BTreeSet::<PathBuf>::from_iter(
            request.coordinate_set().underlying().iter().filter_map(
                |coordinate| match coordinate {
                    Coordinate::Directory(inner) => Some(PathBuf::from(inner)),
                    _ => unreachable!(),
                },
            ),
        );

        Ok(ResolutionResult::from(directories))
    }
}
