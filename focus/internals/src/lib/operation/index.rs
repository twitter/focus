use std::borrow::Borrow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use focus_util::app::{App, ExitCode};
use focus_util::paths::assert_focused_repo;
use tracing::{debug, info};

use crate::coordinate::{Coordinate, CoordinateSet};
use crate::index::{
    get_files_to_materialize, DependencyKey, HashContext, ObjectDatabase, PathsToMaterializeResult,
    RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use crate::model::layering::{LayerSets, RichLayerSet};
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
        Backend::RocksDb => Box::new(RocksDBCache::new(repo)),
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
        DependencyKey::BazelPackage(label) | DependencyKey::BazelBuildFile(label) => {
            format!("bazel:{}", label)
        }

        DependencyKey::Path(path) => format!("directory:{}", path.display()),

        DependencyKey::DummyForTesting(inner_dep_key) => {
            panic!(
                "Cannot convert dummy testing key into coordinate: {:?}",
                inner_dep_key
            );
        }
    }
}

fn resolve_coordinates(
    app: Arc<App>,
    backend: Backend,
    coordinates: HashSet<Coordinate>,
) -> anyhow::Result<ExitCode> {
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
            let num_applied_patterns = repo.sync(&coordinate_set, app, odb.borrow())?;
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

pub fn resolve(
    app: Arc<App>,
    backend: Backend,
    coordinates_or_layers: Vec<String>,
) -> anyhow::Result<ExitCode> {
    let sparse_repo = Path::new(".");
    assert_focused_repo(&sparse_repo)?;

    let all_layers = LayerSets::new(sparse_repo);
    let all_layers = all_layers.available_layers()?;
    let all_layers = RichLayerSet::new(all_layers)?;
    let coordinates: HashSet<Coordinate> = coordinates_or_layers
        .into_iter()
        .flat_map(|coordinate| match all_layers.get(&coordinate) {
            Some(layer) => {
                let coordinates = layer.coordinates();
                info!(
                    num_coordinates = ?coordinates.len(),
                    layer = ?layer.name(),
                    "Num expanded coordinates for layer"
                );
                debug!(?coordinates, layer = ?layer.name(), "Expanded coordinates for layer");
                coordinates.to_vec()
            }
            None => vec![coordinate],
        })
        .map(|coordinate| Coordinate::try_from(coordinate.as_str()))
        .collect::<Result<_, _>>()?;
    resolve_coordinates(app, backend, coordinates)
}

pub fn generate(app: Arc<App>, backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    let all_layers = LayerSets::new(&sparse_repo).available_layers()?;
    let all_coordinates: HashSet<Coordinate> = all_layers
        .layers()
        .iter()
        .flat_map(|layer| layer.coordinates())
        .map(|coordinate| Coordinate::try_from(coordinate.as_str()))
        .collect::<Result<_, _>>()?;
    resolve_coordinates(app, backend, all_coordinates)
}
