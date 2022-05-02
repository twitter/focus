use std::borrow::Borrow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use focus_util::app::{App, ExitCode};
use focus_util::paths::assert_focused_repo;
use tracing::{debug, info};

use crate::target::{Target, TargetSet};
use crate::index::{
    get_files_to_materialize, DependencyKey, HashContext, ObjectDatabase, PathsToMaterializeResult,
    RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use crate::model::project::{ProjectSets, RichProjectSet};
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
                "Cannot convert dummy testing key into target: {:?}",
                inner_dep_key
            );
        }
    }
}

fn resolve_targets(
    app: Arc<App>,
    backend: Backend,
    targets: HashSet<Target>,
) -> anyhow::Result<ExitCode> {
    let dep_keys: HashSet<DependencyKey> = targets
        .iter()
        .map(|target| DependencyKey::from(target.clone()))
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
            let coordinate_set = TargetSet::from(targets);
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
    targets_or_projects: Vec<String>,
) -> anyhow::Result<ExitCode> {
    let sparse_repo = Path::new(".");
    assert_focused_repo(sparse_repo)?;

    let all_projects = ProjectSets::new(sparse_repo);
    let all_projects = all_projects.available_projects()?;
    let all_projects = RichProjectSet::new(all_projects)?;
    let targets: HashSet<Target> = targets_or_projects
        .into_iter()
        .flat_map(|target| match all_projects.get(&target) {
            Some(project) => {
                let targets = project.targets();
                info!(
                    num_coordinates = ?targets.len(),
                    project = ?project.name(),
                    "Num expanded targets for project"
                );
                debug!(?targets, project = ?project.name(), "Expanded targets for project");
                targets.to_vec()
            }
            None => vec![target],
        })
        .map(|target| Target::try_from(target.as_str()))
        .collect::<Result<_, _>>()?;
    resolve_targets(app, backend, targets)
}

pub fn generate(app: Arc<App>, backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    let all_projects = ProjectSets::new(&sparse_repo).available_projects()?;
    let all_targets: HashSet<Target> = all_projects
        .projects()
        .iter()
        .flat_map(|project| project.targets())
        .map(|target| Target::try_from(target.as_str()))
        .collect::<Result<_, _>>()?;
    resolve_targets(app, backend, all_targets)
}
