use std::borrow::Borrow;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use content_addressed_cache::{Cache, CacheSynchronizer, GitBackedCacheSynchronizer, KeysetID};
use focus_util::app::{App, ExitCode};
use focus_util::git_helper;
use focus_util::paths::assert_focused_repo;
use tracing::{debug, debug_span, info};

use crate::index::{
    content_hash_dependency_key, get_files_to_materialize, ContentHash, DependencyKey, HashContext,
    ObjectDatabase, PathsToMaterializeResult, RocksDBCache, RocksDBMemoizationCacheExt,
    FUNCTION_ID,
};
use crate::model::configuration::IndexConfig;
use crate::model::repo::Repo;
use crate::model::selection::OperationAction;
use crate::target::{Target, TargetSet};

const PARENTS_TO_TRY_IN_FETCH: u32 = 100;

pub fn clear(sparse_repo_path: PathBuf) -> anyhow::Result<()> {
    let repo = git2::Repository::open(sparse_repo_path).context("opening sparse repo")?;
    let odb = RocksDBCache::new(&repo);
    let cache: &dyn Cache = &odb;
    cache.clear()?;
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
    sparse_repo_path: &Path,
    targets: HashSet<Target>,
    break_on_missing_keys: bool,
) -> anyhow::Result<Result<ResolveTargetResult, ExitCode>> {
    let dep_keys: HashSet<DependencyKey> = targets
        .iter()
        .map(|target| DependencyKey::from(target.clone()))
        .collect();

    let repo = git2::Repository::open(sparse_repo_path).context("opening sparse repo")?;
    let head_commit = git_helper::get_head_commit(&repo)?;
    let head_tree = head_commit.tree().context("resolving HEAD to tree")?;
    let ctx = HashContext {
        repo: &repo,
        head_tree: &head_tree,
        caches: Default::default(),
    };
    let odb = RocksDBCache::new(&repo);

    let borrowed_odb = odb.borrow();
    let materialize_result = get_files_to_materialize(&ctx, borrowed_odb, dep_keys.clone())?;
    match materialize_result {
        PathsToMaterializeResult::Ok { seen_keys, paths } => {
            Ok(Ok(ResolveTargetResult { seen_keys, paths }))
        }

        PathsToMaterializeResult::MissingKeys {
            seen_keys: _,
            missing_keys,
        } => {
            println!("Missing keys:");
            for (key, hash) in missing_keys {
                println!("{} {}", hash, dep_key_to_target(&key));
            }

            let repo = Repo::open(repo.path(), app.clone())?;
            let (pattern_count, _checked_out) = repo.sync(
                &targets,
                true,
                &repo.config().index,
                app.clone(),
                borrowed_odb,
            )?;
            println!("Pattern count: {}", pattern_count);

            match get_files_to_materialize(&ctx, borrowed_odb, dep_keys)? {
                PathsToMaterializeResult::Ok { seen_keys, paths } => Ok(Ok(ResolveTargetResult {
                    seen_keys,
                    paths: paths.into_iter().collect(),
                })),

                PathsToMaterializeResult::MissingKeys {
                    seen_keys: _,
                    missing_keys,
                } => {
                    println!("Keys STILL missing, this is a bug:");
                    for (key, hash) in missing_keys {
                        println!("{} {}", hash, dep_key_to_target(&key));
                    }

                    if break_on_missing_keys {
                        println!("Breaking for debugging...");
                        println!("Sandbox path: {}", app.sandbox().path().display());
                        drop(odb);
                        loop {
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }

                    Ok(Err(ExitCode(1)))
                }
            }
        }
    }
}

pub fn resolve(
    app: Arc<App>,
    sparse_repo_path: &Path,
    projects_and_targets: Vec<String>,
    break_on_missing_keys: bool,
) -> anyhow::Result<ExitCode> {
    assert_focused_repo(sparse_repo_path)?;
    let repo = Repo::open(sparse_repo_path, app.clone())?;
    let selection = {
        let mut selections = repo.selection_manager()?;
        selections.mutate(OperationAction::Add, &projects_and_targets)?;
        selections.computed_selection()
    }?;
    let targets = TargetSet::try_from(&selection)?;

    let paths = match resolve_targets(app, sparse_repo_path, targets, break_on_missing_keys)? {
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

pub fn hash(
    _app: Arc<App>,
    sparse_repo_path: &Path,
    targets: &[String],
) -> anyhow::Result<ExitCode> {
    let repo = git2::Repository::open(sparse_repo_path)?;
    let head_commit = git_helper::get_head_commit(&repo)?;
    let head_tree = head_commit.tree()?;
    let hash_context = HashContext {
        repo: &repo,
        head_tree: &head_tree,
        caches: Default::default(),
    };
    info!(?hash_context, "Using this hash context");

    for target in targets {
        let target = Target::try_from(target.as_str())?;
        let dep_key = DependencyKey::from(target);
        let hash = content_hash_dependency_key(&hash_context, &dep_key, &mut HashSet::new())?;
        println!("{hash} {dep_key:?}");
    }

    debug!(?hash_context, "Finished with this hash context");

    Ok(ExitCode(0))
}

pub fn get(_app: Arc<App>, sparse_repo_path: &Path, hash: &str) -> anyhow::Result<ExitCode> {
    let repo = git2::Repository::open(sparse_repo_path)?;
    let hash = ContentHash::from_str(hash)?;
    let odb = RocksDBCache::new(&repo);
    let value = odb.get_direct(&hash)?;
    match value {
        Some(value) => {
            println!("{hash} {value:#?}");
            Ok(ExitCode(0))
        }
        None => {
            println!("{hash} <not found>");
            Ok(ExitCode(1))
        }
    }
}

pub fn generate(
    app: Arc<App>,

    sparse_repo_path: PathBuf,
    break_on_missing_keys: bool,
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
    match resolve_targets(app, &sparse_repo_path, all_targets, break_on_missing_keys)? {
        Ok(_result) => Ok(ExitCode(0)),
        Err(exit_code) => Ok(exit_code),
    }
}

fn index_repo_dir(sparse_repo_path: &Path) -> PathBuf {
    sparse_repo_path.join(".git").join("focus").join("index")
}

pub const INDEX_DEFAULT_REMOTE: &str = "https://git.twitter.biz/focus-index";

pub fn fetch(app: Arc<App>, sparse_repo_path: PathBuf) -> anyhow::Result<ExitCode> {
    let repo = Repo::open(&sparse_repo_path, app.clone())
        .with_context(|| format!("Opening repository at {}", &sparse_repo_path.display()))?;
    let cache = RocksDBCache::new(repo.underlying());
    fetch_internal(app, &cache, sparse_repo_path, &repo.config().index)
}

pub(crate) fn fetch_internal(
    app: Arc<App>,
    cache: &RocksDBCache,
    sparse_repo_path: PathBuf,
    index_config: &IndexConfig,
) -> anyhow::Result<ExitCode> {
    if !index_config.enabled {
        return Ok(ExitCode(0));
    }

    let index_dir = index_repo_dir(&sparse_repo_path);
    let synchronizer =
        GitBackedCacheSynchronizer::create(index_dir, index_config.remote.clone(), app.clone())?;
    let repo = Repo::open(sparse_repo_path.as_path(), app).context("Failed to open repo")?;
    let mut commit = repo.get_head_commit()?;

    let available_keysets = synchronizer.available_remote_keysets()?;

    let mut found_keyset: Option<KeysetID> = None;
    for _ in 0..PARENTS_TO_TRY_IN_FETCH {
        let keyset_id = commit.tree()?.id();

        if available_keysets.contains(&keyset_id) {
            found_keyset = Some(keyset_id);
            break;
        }
        commit = commit.parent(0)?;
    }
    if let Some(keyset_id) = found_keyset {
        let keyset_id_str = keyset_id.to_string();
        let span = debug_span!("Fetching index");
        info!(tag = %keyset_id_str, "Fetching index");
        let _guard = span.enter();
        synchronizer
            .fetch_and_populate(keyset_id, cache)
            .context("Fetching index data")?;
    } else {
        info!("No index matches the current commit");
    }

    Ok(ExitCode(0))
}

pub fn push(
    app: Arc<App>,
    sparse_repo_path: PathBuf,
    remote: String,
    break_on_missing_keys: bool,
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

    let index_dir = index_repo_dir(&sparse_repo_path);
    std::fs::create_dir_all(&index_dir).context("creating index directory")?;
    let synchronizer = GitBackedCacheSynchronizer::create(index_dir, remote, app.clone())?;

    let head_commit = repo.get_head_commit()?;
    let head_tree = head_commit.tree().context("finding HEAD tree")?;
    let ctx = HashContext {
        repo: repo.underlying(),
        head_tree: &head_tree,
        caches: Default::default(),
    };

    let ResolveTargetResult {
        seen_keys,
        paths: _,
    } = match resolve_targets(app, &sparse_repo_path, all_targets, break_on_missing_keys)? {
        Ok(result) => result,
        Err(exit_code) => return Ok(exit_code),
    };

    let odb = RocksDBCache::new(repo.underlying());
    let cache: &dyn ObjectDatabase = &odb;

    let keyset = {
        let mut result = HashSet::new();
        for key in seen_keys {
            let (hash, value) = cache.get(&ctx, &key)?;
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

#[cfg(test)]
mod tests {
    use focus_testing::ScratchGitRepo;
    use maplit::hashset;

    use crate::model::configuration::{Configuration, INDEX_CONFIG_FILENAME};
    use crate::model::selection::store_model;
    use crate::operation::testing::integration::RepoPairFixture;
    use crate::target::Label;

    use super::*;

    #[test]
    fn test_index_push_and_fetch() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let remote_index_store = ScratchGitRepo::new_static_fixture(temp_dir.path())?;
        let remote = format!("file://{}", remote_index_store.path().display());

        let app = Arc::new(App::new_for_testing()?);
        let label: Label = "//project_a/src/main/java/com/example/cmdline:runner".parse()?;

        // Populate remote index store.
        {
            let fixture = RepoPairFixture::new()?;
            fixture.perform_clone()?;
            let ExitCode(exit_code) = push(
                app.clone(),
                fixture.sparse_repo_path.clone(),
                remote.clone(),
                false,
            )?;
            assert_eq!(exit_code, 0);
        }

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let index_config = IndexConfig {
            enabled: true,
            remote,
        };
        let config_dir = Configuration::config_dir(&fixture.sparse_repo_path);
        std::fs::create_dir_all(&config_dir)?;
        store_model(config_dir.join(INDEX_CONFIG_FILENAME), &index_config)?;

        let repo = fixture.sparse_repo()?;
        let repo = repo.underlying();
        let head_tree = repo.head()?.peel_to_commit()?.tree()?;
        let ctx = HashContext {
            repo,
            head_tree: &head_tree,
            caches: Default::default(),
        };

        // Try to materialize files -- this should be a cache miss.
        {
            let odb = RocksDBCache::new(repo);
            let materialize_result = get_files_to_materialize(
                &ctx,
                odb.borrow(),
                hashset! {DependencyKey::BazelPackage(label.clone() )},
            )?;
            insta::assert_debug_snapshot!(materialize_result, @r###"
            MissingKeys {
                missing_keys: {
                    (
                        BazelPackage(
                            Label("//project_a/src/main/java/com/example/cmdline:runner"),
                        ),
                        ContentHash(
                            c31c95867b703cb343a04d99a8909eb17f8ba9ee,
                        ),
                    ),
                },
                seen_keys: {
                    BazelPackage(
                        Label("//project_a/src/main/java/com/example/cmdline:runner"),
                    ),
                },
            }
            "###);
        }

        let ExitCode(exit_code) = fetch(app, fixture.sparse_repo_path.clone())?;
        assert_eq!(exit_code, 0);

        // Try to materialize files again -- this should be a cache hit.
        {
            let odb = RocksDBCache::new(repo);
            let materialize_result = get_files_to_materialize(
                &ctx,
                odb.borrow(),
                hashset! {DependencyKey::BazelPackage(label )},
            )?;
            insta::assert_debug_snapshot!(materialize_result, @r###"
            Ok {
                seen_keys: {
                    BazelPackage(
                        Label("//library_a:a"),
                    ),
                    BazelPackage(
                        Label("//project_a/src/main/java/com/example/cmdline:Runner.java"),
                    ),
                    BazelPackage(
                        Label("//project_a/src/main/java/com/example/cmdline:runner"),
                    ),
                },
                paths: {
                    "library_a",
                    "project_a/src/main/java/com/example/cmdline",
                },
            }
            "###);
        }

        Ok(())
    }
}
