use std::{
    iter::FromIterator,
    path::{Path, PathBuf},
};

use super::*;

/// Resolves directories verbatim
pub struct DirectoryResolver {
    #[allow(unused)]
    cache_root: PathBuf,
}

impl Resolver for DirectoryResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            cache_root: cache_root.join("directory"),
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
