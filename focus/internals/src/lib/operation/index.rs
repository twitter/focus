use std::borrow::Borrow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use focus_util::app::{App, ExitCode};
use focus_util::git_helper::get_head_commit;
use focus_util::paths::assert_focused_repo;

use crate::index::{
    get_files_to_materialize, DependencyKey, HashContext, ObjectDatabase, PathsToMaterializeResult,
    RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use crate::model::repo::Repo;
use crate::model::selection::OperationAction;
use crate::target::{Target, TargetSet};

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

fn dep_key_to_target(dep_key: &DependencyKey) -> String {
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
    let head_commit = get_head_commit(&repo)?;
    let head_tree = head_commit.tree().context("resolving HEAD to tree")?;
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
                println!("{} {}", hash, dep_key_to_target(&key));
            }

            let repo = Repo::open(repo.path(), app.clone())?;
            let (num_applied_patterns, _checked_out) = repo.sync(&targets, app, odb.borrow())?;
            println!("Applied patterns: {}", num_applied_patterns);

            match get_files_to_materialize(&ctx, odb.borrow(), dep_keys)? {
                PathsToMaterializeResult::Ok { .. } => Ok(ExitCode(0)),

                PathsToMaterializeResult::MissingKeys { keys } => {
                    println!("Keys STILL missing, this is a bug:");
                    for (key, hash) in keys {
                        println!("{} {}", hash, dep_key_to_target(&key));
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
    projects_and_targets: Vec<String>,
) -> anyhow::Result<ExitCode> {
    let sparse_repo = Path::new(".");
    assert_focused_repo(sparse_repo)?;
    let repo = Repo::open(sparse_repo, app.clone())?;
    let selection = {
        let mut selections = repo.selection_manager()?;
        selections.mutate(OperationAction::Add, &projects_and_targets)?;
        selections.computed_selection()
    }?;
    let targets = TargetSet::try_from(&selection)?;
    resolve_targets(app, backend, targets)
}

pub fn generate(app: Arc<App>, backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo, app.clone())?;
    let selections = repo.selection_manager()?;
    let targets = {
        let mut targets = TargetSet::try_from(&selections.project_catalog().mandatory_projects)?;
        targets.extend(TargetSet::try_from(
            &selections.project_catalog().optional_projects,
        )?);
        targets
    };

    resolve_targets(app, backend, targets)
}
