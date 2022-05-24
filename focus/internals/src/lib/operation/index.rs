use std::borrow::Borrow;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use content_addressed_cache::{CacheSynchronizer, GitBackedCacheSynchronizer};
use focus_util::app::{App, ExitCode};
use focus_util::git_helper;
use focus_util::paths::assert_focused_repo;
use tracing::{debug, info};

use crate::index::{
    content_hash_dependency_key, get_files_to_materialize, DependencyKey, HashContext,
    ObjectDatabase, PathsToMaterializeResult, RocksDBCache, RocksDBMemoizationCacheExt,
    SimpleGitOdb, FUNCTION_ID,
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

pub fn clear(backend: Backend, sparse_repo_path: PathBuf) -> anyhow::Result<()> {
    let repo = git2::Repository::open(sparse_repo_path).context("opening sparse repo")?;
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
            let (pattern_count, _checked_out) =
                repo.sync(&targets, true, app.clone(), odb.borrow())?;
            println!("Pattern count: {}", pattern_count);

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
    backend: Backend,
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

    let paths = match resolve_targets(
        app,
        backend,
        sparse_repo_path,
        targets,
        break_on_missing_keys,
    )? {
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
        let hash = content_hash_dependency_key(&hash_context, &dep_key)?;
        println!("{hash} {dep_key:?}");
    }

    debug!(?hash_context, "Finished with this hash context");

    Ok(ExitCode(0))
}

pub fn generate(
    app: Arc<App>,
    backend: Backend,
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

    match resolve_targets(
        app,
        backend,
        &sparse_repo_path,
        all_targets,
        break_on_missing_keys,
    )? {
        Ok(_result) => Ok(ExitCode(0)),
        Err(exit_code) => Ok(exit_code),
    }
}

fn index_repo_dir(sparse_repo_path: &Path) -> PathBuf {
    sparse_repo_path.join(".git").join("focus").join("index")
}

pub const INDEX_DEFAULT_REMOTE: &str = "https://git.twitter.biz/focus-index";

pub fn fetch(
    app: Arc<App>,
    backend: Backend,
    sparse_repo_path: PathBuf,
    remote: String,
) -> anyhow::Result<ExitCode> {
    let index_dir = index_repo_dir(&sparse_repo_path);
    let synchronizer = GitBackedCacheSynchronizer::create(index_dir, remote, app)?;

    let repo = git2::Repository::open(&sparse_repo_path)?;
    let odb = match backend {
        Backend::Simple => {
            anyhow::bail!("Backend not supported, as it does not implement `Cache`: {backend:?}")
        }
        Backend::RocksDb => RocksDBCache::new(&repo),
    };
    synchronizer
        .fetch_and_populate(&odb)
        .context("Fetching index data")?;

    Ok(ExitCode(0))
}

pub fn push(
    app: Arc<App>,
    backend: Backend,
    sparse_repo_path: PathBuf,
    remote: String,
    additional_ref_name: Option<&str>,
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
    } = match resolve_targets(
        app,
        backend.clone(),
        &sparse_repo_path,
        all_targets,
        break_on_missing_keys,
    )? {
        Ok(result) => result,
        Err(exit_code) => return Ok(exit_code),
    };

    let odb = match backend {
        Backend::Simple => {
            anyhow::bail!("Backend not supported, as it does not implement `Cache`: {backend:?}")
        }
        Backend::RocksDb => RocksDBCache::new(repo.underlying()),
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
    synchronizer.share(ctx.head_tree.id(), &keyset, &odb, None, additional_ref_name)?;

    Ok(ExitCode(0))
}

#[cfg(test)]
mod tests {
    use focus_testing::ScratchGitRepo;
    use maplit::hashset;

    use crate::operation::testing::integration::RepoPairFixture;
    use crate::target::Label;

    use super::*;

    #[test]
    fn test_index_push_and_fetch() -> anyhow::Result<()> {
        let backend = Backend::RocksDb;

        let temp_dir = tempfile::tempdir()?;
        let remote_index_store = ScratchGitRepo::new_static_fixture(temp_dir.path())?;
        let remote = format!("file://{}", remote_index_store.path().display());

        let app = Arc::new(App::new(false)?);
        let label: Label = "//project_a/src/main/java/com/example/cmdline:runner".parse()?;

        // Populate remote index store.
        {
            let fixture = RepoPairFixture::new()?;
            fixture.perform_clone()?;
            let ExitCode(exit_code) = push(
                app.clone(),
                backend.clone(),
                fixture.sparse_repo_path.clone(),
                remote.clone(),
                None,
                false,
            )?;
            assert_eq!(exit_code, 0);
        }

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;
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
            let odb = make_odb(backend.clone(), repo);
            let materialize_result = get_files_to_materialize(
                &ctx,
                odb.borrow(),
                hashset! {DependencyKey::BazelPackage(label.clone() )},
            )?;
            insta::assert_debug_snapshot!(materialize_result, @r###"
            MissingKeys {
                keys: {
                    (
                        BazelPackage(
                            Label("//project_a/src/main/java/com/example/cmdline:runner"),
                        ),
                        ContentHash(
                            ca3400ca51b0961f57a8bac6f8b5280dc02012ef,
                        ),
                    ),
                },
            }
            "###);
        }

        let ExitCode(exit_code) = fetch(
            app.clone(),
            backend.clone(),
            fixture.sparse_repo_path.clone(),
            remote.clone(),
        )?;
        assert_eq!(exit_code, 0);

        // Try to materialize files again -- this should be a cache hit.
        {
            let odb = make_odb(backend.clone(), repo);
            let materialize_result = get_files_to_materialize(
                &ctx,
                odb.borrow(),
                hashset! {DependencyKey::BazelPackage(label.clone() )},
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

    #[test]
    fn test_index_push_additional_ref_name() -> anyhow::Result<()> {
        let backend = Backend::RocksDb;

        let temp_dir = tempfile::tempdir()?;
        let remote_index_store = ScratchGitRepo::new_static_fixture(temp_dir.path())?;
        let remote = format!("file://{}", remote_index_store.path().display());

        let app = Arc::new(App::new(false)?);

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;
        let ExitCode(exit_code) = push(
            app.clone(),
            backend.clone(),
            fixture.sparse_repo_path.clone(),
            remote.clone(),
            Some("latest"),
            false,
        )?;
        assert_eq!(exit_code, 0);

        let ExitCode(exit_code) = fetch(
            app.clone(),
            backend.clone(),
            fixture.sparse_repo_path.clone(),
            remote.clone(),
        )?;
        assert_eq!(exit_code, 0);

        Ok(())
    }
}
