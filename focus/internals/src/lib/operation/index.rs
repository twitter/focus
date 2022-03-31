use std::borrow::Borrow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use focus_util::app::{App, ExitCode};

use crate::coordinate::{Coordinate, CoordinateSet};
use crate::index::{
    get_files_to_materialize, DependencyKey, HashContext, ObjectDatabase, PathsToMaterializeResult,
    RocksDBMemoizationCache, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use crate::model::repo::Repo;

#[derive(
    Clone,
    Debug,
    clap::ArgEnum,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
    strum_macros::EnumIter,
)]
#[strum(serialize_all = "kebab-case")]
pub enum Backend {
    /// Use `SimpleGitOdb` as the back-end. Not for production use.
    Simple,

    /// Use RocksDB an the back-end.
    RocksDb,
}

fn make_odb<'a>(backend: Backend, repo: &'a git2::Repository) -> Box<dyn ObjectDatabase + 'a> {
    match backend {
        Backend::Simple => Box::new(SimpleGitOdb::new(repo)),
        Backend::RocksDb => Box::new(RocksDBMemoizationCache::new(repo)),
    }
}

pub fn clear(backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<()> {
    let repo = git2::Repository::open(sparse_repo).context("opening sparse repo")?;
    let odb = make_odb(backend, &repo);
    odb.clear()?;
    Ok(())
}

fn dep_key_to_coordinate(dep_key: &DependencyKey) -> String {
    match dep_key {
        DependencyKey::BazelPackage {
            external_repository: None,
            path,
        } => format!("bazel://{}", path.display()),

        DependencyKey::BazelPackage {
            external_repository: Some(external_repository),
            path,
        } => format!("bazel:@{}//{}", external_repository, path.display()),

        DependencyKey::BazelBuildFile(label) => format!("bazel:{}", label),

        DependencyKey::Path(path) => format!("directory:{}", path.display()),

        DependencyKey::DummyForTesting(inner_dep_key) => {
            panic!(
                "Cannot convert dummy testing key into coordinate: {:?}",
                inner_dep_key
            );
        }
    }
}

pub fn resolve(
    app: Arc<App>,
    backend: Backend,
    coordinates: Vec<String>,
) -> anyhow::Result<ExitCode> {
    let coordinates: HashSet<Coordinate> = coordinates
        .into_iter()
        .map(|coordinate| Coordinate::try_from(coordinate.as_str()))
        .collect::<Result<_, _>>()?;
    let dep_keys: HashSet<DependencyKey> = coordinates
        .iter()
        .map(|coordinate| DependencyKey::from(coordinate.clone()))
        .collect();

    let repo = git2::Repository::open(".").context("opening sparse repo")?;
    let head_commit = repo.head().context("looking up HEAD")?;
    let head_tree = head_commit
        .peel_to_tree()
        .context("resolving HEAD to tree")?;
    let ctx = HashContext {
        repo: &repo,
        head_tree: &head_tree,
        caches: Default::default(),
    };
    let odb = make_odb(backend, &repo);

    let materialize_result = get_files_to_materialize(&ctx, odb.borrow(), dep_keys.clone())?;
    match materialize_result {
        PathsToMaterializeResult::Ok { paths } => {
            println!("Paths to materialize:");
            for path in paths {
                println!("{}", path.display());
            }
            Ok(ExitCode(0))
        }

        PathsToMaterializeResult::MissingKeys { keys } => {
            println!("Missing keys:");
            for (key, hash) in keys {
                println!("{} {}", hash, dep_key_to_coordinate(&key));
            }

            let repo = Repo::open(repo.path(), app.clone())?;
            let coordinate_set = CoordinateSet::from(coordinates);
            let num_applied_patterns = repo.sync(&coordinate_set, app)?;
            println!("Applied patterns: {}", num_applied_patterns);

            match get_files_to_materialize(&ctx, odb.borrow(), dep_keys)? {
                PathsToMaterializeResult::Ok { .. } => Ok(ExitCode(0)),

                PathsToMaterializeResult::MissingKeys { keys } => {
                    println!("Keys STILL missing, this is a bug:");
                    for (key, hash) in keys {
                        println!("{} {}", hash, dep_key_to_coordinate(&key));
                    }
                    Ok(ExitCode(1))
                }
            }
        }
    }
}
