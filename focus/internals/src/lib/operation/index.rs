use std::borrow::Borrow;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use content_addressed_cache::{CacheSynchronizer, GitBackedCacheSynchronizer};
use focus_util::app::{App, ExitCode};
use focus_util::git_helper::get_head_commit;
use focus_util::paths::assert_focused_repo;
use tracing::info;

use crate::index::{
    get_files_to_materialize, DependencyKey, HashContext, ObjectDatabase, PathsToMaterializeResult,
    RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb, FUNCTION_ID,
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

#[derive(Clone, Debug)]
struct ResolveTargetResult {
    seen_keys: BTreeSet<DependencyKey>,
    paths: BTreeSet<PathBuf>,
}

fn resolve_targets(
    app: Arc<App>,
    backend: Backend,
    targets: HashSet<Target>,
) -> anyhow::Result<Result<ResolveTargetResult, ExitCode>> {
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
        PathsToMaterializeResult::Ok { seen_keys, paths } => {
            Ok(Ok(ResolveTargetResult { seen_keys, paths }))
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
                PathsToMaterializeResult::Ok { seen_keys, paths } => Ok(Ok(ResolveTargetResult {
                    seen_keys,
                    paths: paths.into_iter().collect(),
                })),

                PathsToMaterializeResult::MissingKeys { keys } => {
                    println!("Keys STILL missing, this is a bug:");
                    for (key, hash) in keys {
                        println!("{} {}", hash, dep_key_to_target(&key));
                    }
                    Ok(Err(ExitCode(1)))
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

    let paths = match resolve_targets(app, backend, targets)? {
        Ok(ResolveTargetResult {
            seen_keys: _,
            paths,
        }) => paths,
        Err(exit_code) => return Ok(exit_code),
    };

    println!("Paths to materialize:");
    for path in paths.iter() {
        println!("{}", path.display());
    }
    Ok(ExitCode(0))
}

pub fn generate(app: Arc<App>, backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo, app.clone())?;
    let selections = repo.selection_manager()?;
    let all_targets = {
        let mut targets = TargetSet::try_from(&selections.project_catalog().mandatory_projects)?;
        targets.extend(TargetSet::try_from(
            &selections.project_catalog().optional_projects,
        )?);
        targets
    };

    match resolve_targets(app, backend, all_targets)? {
        Ok(_result) => Ok(ExitCode(0)),
        Err(exit_code) => Ok(exit_code),
    }
}

fn get_index_dir(sparse_repo: &Path) -> PathBuf {
    sparse_repo.join(".git").join("focus").join("focus-index")
}

const INDEX_REMOTE: &str = "https://git.twitter.biz/focus-index";

pub fn fetch(_app: Arc<App>, _backend: Backend, _sparse_repo: PathBuf) -> anyhow::Result<ExitCode> {
    todo!()
}

pub fn push(
    app: Arc<App>,
    backend: Backend,
    sparse_repo_path: PathBuf,
) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo_path, app.clone())?;
    let selections = repo.selection_manager()?;
    let all_targets = {
        let mut targets = TargetSet::try_from(&selections.project_catalog().mandatory_projects)?;
        targets.extend(TargetSet::try_from(
            &selections.project_catalog().optional_projects,
        )?);
        targets
    };

    let index_dir = get_index_dir(&sparse_repo_path);
    std::fs::create_dir_all(&index_dir).context("creating index directory")?;
    let synchronizer =
        GitBackedCacheSynchronizer::create(index_dir, INDEX_REMOTE.to_string(), app.clone())?;

    let head_commit = repo.get_head_commit()?;
    let head_tree = head_commit.tree().context("finding HEAD tree")?;
    let ctx = HashContext {
        repo: &repo.underlying(),
        head_tree: &head_tree,
        caches: Default::default(),
    };

    let ResolveTargetResult {
        seen_keys,
        paths: _,
    } = match resolve_targets(app, backend.clone(), all_targets)? {
        Ok(result) => result,
        Err(exit_code) => return Ok(exit_code),
    };

    let odb = match backend {
        Backend::Simple => {
            anyhow::bail!("Backend not supported, as it does not implement `Cache`: {backend:?}")
        }
        Backend::RocksDb => RocksDBCache::new(&repo.underlying()),
    };

    let keyset = {
        let mut result = HashSet::new();
        for key in seen_keys {
            let (hash, value) = odb.get(&ctx, &key)?;
            if value.is_none() {
                panic!("Value not found for key: {key:?}");
            }
            result.insert((*FUNCTION_ID, git2::Oid::from(hash)));
        }
        result
    };

    info!("Pushing index");
    synchronizer.share(ctx.head_tree.id(), &keyset, &odb, None)?;

    Ok(ExitCode(0))
}
