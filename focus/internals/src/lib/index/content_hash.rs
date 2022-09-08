// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeSet, HashMap};
use std::fmt::{Display, Write};
use std::hash::Hash;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use futures::executor::{block_on, ThreadPool, ThreadPoolBuilder};
use futures::future::{try_join_all, BoxFuture, Fuse, Shared};
use futures::task::SpawnExt;
use futures::FutureExt;
use lazy_static::lazy_static;
use ouroboros::self_referencing;
use regex::Regex;
use thiserror::Error;
use tracing::debug;
use tracing::error;
use tracing::trace;
use tracing::warn;

use crate::target::Label;
use crate::target::TargetName;
use focus_util::paths::is_relevant_to_build_graph;

use super::DependencyKey;

/// This value is mixed into all content hashes. Update this value when
/// content-hashing changes in a backward-incompatible way.
const VERSION: usize = 7;

/// The hash of a [`DependencyKey`]'s syntactic content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub(super) git2::Oid);

impl From<ContentHash> for git2::Oid {
    fn from(hash: ContentHash) -> Self {
        let ContentHash(oid) = hash;
        oid
    }
}

impl Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(hash) = self;
        write!(f, "{}", hash)
    }
}

impl FromStr for ContentHash {
    type Err = git2::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let oid = git2::Oid::from_str(s)?;
        Ok(ContentHash(oid))
    }
}

#[derive(Debug, Default)]
pub struct Caches {
    /// Cache of hashed dependency keys. These are only valid for the provided `repo`/`head_tree`.
    dependency_key_cache:
        HashMap<DependencyKey, Shared<Fuse<BoxFuture<'static, Result<ContentHash>>>>>,

    /// Cache of hashed tree paths, which should be either:
    ///
    ///   - Bazel-relevant build files.
    ///   - Directories.
    ///   - Non-existent.
    ///
    /// These are only valid for the provided `repo`/`head_tree`.
    tree_path_cache: HashMap<PathBuf, ContentHash>,

    /// Cache of parsed load dependencies. The OIDs here are tree OIDs which
    /// have to be traversed to find any files relevant to the build graph.
    load_dependencies_cache: HashMap<git2::Oid, BTreeSet<Label>>,

    /// Cache of dependencies loaded from the `prelude_bazel` file.
    prelude_deps_cache: Option<BTreeSet<Label>>,
}

#[self_referencing]
struct RepoState {
    repo: git2::Repository,
    #[borrows(repo)]
    #[covariant]
    head_tree: git2::Tree<'this>,
}

unsafe impl Send for RepoState {}

impl std::fmt::Debug for RepoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoState")
            .field("head_tree", &self.borrow_head_tree())
            .field(
                "repo",
                &format!("<repo at path {:?}>", self.borrow_repo().path()),
            )
            .finish()
    }
}

#[derive(Clone, Debug)]
struct RepoPool {
    repo_path: PathBuf,
    head_tree_oid: git2::Oid,
    items: Arc<Mutex<Vec<RepoState>>>,
}

struct RepoStateGuard<'a> {
    pool: &'a RepoPool,
    inner: Option<RepoState>,
}

impl Deref for RepoStateGuard<'_> {
    type Target = RepoState;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl Drop for RepoStateGuard<'_> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            self.pool.dealloc(inner);
        }
    }
}

impl RepoPool {
    fn create(&self) -> Result<RepoStateGuard> {
        let inner = self.alloc()?;
        let guard = RepoStateGuard {
            pool: self,
            inner: Some(inner),
        };
        Ok(guard)
    }

    fn alloc(&self) -> Result<RepoState> {
        let mut items = self.items.lock().unwrap();
        match items.pop() {
            Some(item) => Ok(item),
            None => {
                let repo =
                    git2::Repository::open(&self.repo_path).map_err(Error::CloneRepository)?;
                let repo_state = RepoStateTryBuilder {
                    repo,
                    head_tree_builder: |repo| {
                        repo.find_tree(self.head_tree_oid).map_err(Error::ReadTree)
                    },
                }
                .try_build()?;
                Ok(repo_state)
            }
        }
    }

    fn dealloc(&self, repo_state: RepoState) {
        let mut items = self.items.lock().unwrap();
        items.push(repo_state);
    }
}

/// Context used to compute a content hash.
#[derive(Clone, Debug)]
pub struct HashContext {
    /// The Git repository and head tree state.
    repo_pool: RepoPool,

    /// Associated caches.
    caches: Arc<Mutex<Caches>>,

    thread_pool: ThreadPool,
}

const _: () = {
    fn assert_hash_context_sync<T: Sync>() {}
    fn assert_hash_context_send<T: Send>() {}
    fn assert() {
        assert_hash_context_sync::<HashContext>();
        assert_hash_context_send::<HashContext>();
    }
};

impl HashContext {
    /// Construct a new hash context from the given repository state.
    pub fn new(repo: &git2::Repository, head_tree: &git2::Tree) -> Result<Self> {
        let repo_pool = RepoPool {
            repo_path: repo.path().to_owned(),
            head_tree_oid: head_tree.id(),
            items: Default::default(),
        };
        let thread_pool = ThreadPoolBuilder::new()
            .name_prefix("content-hash-")
            .pool_size(100)
            .create()
            .map_err(Error::CreateThreadPool)?;
        Ok(Self {
            repo_pool,
            caches: Default::default(),
            thread_pool,
        })
    }

    /// Call the provided function with a reference to the underlying
    /// repository.
    pub fn with_repo<T>(&self, f: impl Fn(&git2::Repository) -> T) -> Result<T> {
        // TODO: don't leak repo
        let repo_state = self.repo_pool.create()?;
        let result = f(repo_state.borrow_repo());
        Ok(result)
    }

    /// Call the provided function with a reference to the underlying head tree.
    pub fn with_head_tree<T>(&self, f: impl Fn(&git2::Tree) -> T) -> Result<T> {
        // TODO: don't leak repo
        let repo_state = self.repo_pool.create()?;
        let result = f(repo_state.borrow_head_tree());
        Ok(result)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not create thread pool: {0}")]
    CreateThreadPool(#[source] std::io::Error),

    #[error("could not spawn task: {0}")]
    SpawnTask(String),

    #[error("could not read tree: {0}")]
    ReadTree(#[source] git2::Error),

    #[error("could not read tree entry: {0}")]
    ReadTreeEntry(#[source] git2::Error),

    #[error("could not hash object: {0}")]
    HashObject(#[source] git2::Error),

    #[error("could not make copy of repository object: {0}")]
    CloneRepository(#[source] git2::Error),

    #[error("I/O error: {0}")]
    Fmt(#[from] std::fmt::Error),

    #[error("bug: {0}")]
    Bug(String),
}

fn clone_git_error(error: &git2::Error) -> git2::Error {
    git2::Error::new(error.code(), error.class(), error.message())
}

impl Clone for Error {
    fn clone(&self) -> Self {
        match self {
            Self::CreateThreadPool(e) => Self::CreateThreadPool(std::io::Error::from(e.kind())),
            Self::SpawnTask(message) => Self::SpawnTask(message.clone()),
            Self::ReadTree(e) => Self::ReadTree(clone_git_error(e)),
            Self::ReadTreeEntry(e) => Self::ReadTreeEntry(clone_git_error(e)),
            Self::HashObject(e) => Self::HashObject(clone_git_error(e)),
            Self::CloneRepository(e) => Self::CloneRepository(clone_git_error(e)),
            Self::Fmt(e) => Self::Fmt(*e),
            Self::Bug(message) => Self::Bug(message.clone()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Compute a content-addressable hash for the provided [`DependencyKey`] using
/// the context in `ctx`.
pub fn content_hash(ctx: &HashContext, key: &DependencyKey) -> Result<ContentHash> {
    let ctx = ctx.clone();
    let key = key.clone();
    block_on(content_hash_dependency_key(ctx, key))
}

fn content_hash_dependency_key(
    ctx: HashContext,
    key: DependencyKey,
) -> Shared<Fuse<BoxFuture<'static, Result<ContentHash>>>> {
    let cache_key = match key {
        DependencyKey::BazelPackage(Label {
            external_repository,
            path_components,
            target_name: _,
        }) => DependencyKey::BazelPackage(Label {
            external_repository: external_repository.clone(),
            path_components: path_components.clone(),
            target_name: TargetName::Ellipsis,
        }),
        other @ (DependencyKey::BazelBuildFile(_)
        | DependencyKey::Path(_)
        | DependencyKey::DummyForTesting(_)) => other.clone(),
    };
    let key = cache_key.clone();
    debug!(?key, ?cache_key, "Hashing dependency key");

    let cache = &mut ctx.caches.lock().unwrap().dependency_key_cache;
    if let Some(task) = cache.get(&cache_key) {
        return task.clone();
    }

    let task = content_hash_dependency_key_inner(ctx.clone(), key.clone());
    let task = task.boxed().fuse().shared();
    cache.insert(cache_key.to_owned(), task.clone());
    task
}

async fn content_hash_dependency_key_inner(
    ctx: HashContext,
    key: DependencyKey,
) -> Result<ContentHash> {
    println!("@nocommit hashing {key:?}");
    enum KeyOrPath {
        Key(DependencyKey),
        Path(PathBuf),
    }
    let (kind, maybe_label, values_to_hash) = match &key {
        DependencyKey::BazelPackage(
            label @ Label {
                external_repository,
                path_components,
                target_name: _,
            },
        ) => {
            let path: PathBuf = path_components.iter().collect();
            let mut dep_keys = vec![DependencyKey::Path(path.clone())];

            dep_keys.extend(match external_repository {
                Some(_) => vec![],
                None => {
                    let repo_state = ctx.repo_pool.create()?;
                    let mut loaded_deps = match get_tree_for_path(&repo_state, &path)? {
                        Some(tree) => find_load_dependencies(&ctx, &tree)?,
                        None => Default::default(),
                    };

                    let prelude_deps = get_prelude_deps(&ctx)?;
                    loaded_deps.extend(prelude_deps);
                    loaded_deps
                        .into_iter()
                        .map(DependencyKey::BazelBuildFile)
                        .collect()
                }
            });

            // Every package has an implicit dependency on the `WORKSPACE` file.
            dep_keys.push(DependencyKey::BazelBuildFile(Label {
                external_repository: None,
                path_components: Vec::new(),
                target_name: TargetName::Name("WORKSPACE".to_string()),
            }));

            (
                "BazelPackage",
                Some(label),
                dep_keys.into_iter().map(KeyOrPath::Key).collect(),
            )
        }

        DependencyKey::BazelBuildFile(
            label @ Label {
                external_repository,
                path_components,
                target_name,
            },
        ) => {
            let mut dep_keys = match (external_repository, target_name) {
                (Some(_), _) => Vec::new(),
                (None, TargetName::Ellipsis) => {
                    return Err(Error::Bug(format!(
                        "Got label referring to a ellipsis, but it should be a BUILD or .bzl file: {:?}",
                        label
                    )));
                }

                (None, TargetName::Name(target_name)) => {
                    let path: PathBuf = {
                        let mut path: PathBuf = path_components.iter().collect();
                        path.push(target_name);
                        path
                    };
                    let mut dep_keys = vec![DependencyKey::Path(path.clone())];

                    let loaded_deps = {
                        // TODO: don't leak repo
                        let repo_state = ctx.repo_pool.create()?;
                        match repo_state.borrow_head_tree().get_path(&path) {
                            Ok(tree_entry) => {
                                if is_tree_entry_relevant_to_build_graph(&tree_entry) {
                                    extract_load_statements_from_tree_entry(
                                        &repo_state,
                                        &tree_entry,
                                    )?
                                } else {
                                    Default::default()
                                }
                            }
                            Err(e) if e.code() == git2::ErrorCode::NotFound => Default::default(),
                            Err(e) => return Err(Error::ReadTreeEntry(e)),
                        }
                    };

                    dep_keys.extend(
                        loaded_deps
                            .into_iter()
                            .map(DependencyKey::BazelBuildFile)
                            .filter(|dep_key| &key != dep_key),
                    );

                    dep_keys
                }
            };

            // Every `.bzl` file (or similar) has an implicit dependency on the
            // `WORKSPACE` file. However, the `WORKSPACE` file itself may `load`
            // `.bzl` files in the repository. To avoid a circular dependency,
            // use only the textual hash of the WORKSPACE as the dependency key
            // here.
            dep_keys.push(DependencyKey::Path("WORKSPACE".into()));

            (
                "BazelBuildFile",
                Some(label),
                dep_keys.into_iter().map(KeyOrPath::Key).collect(),
            )
        }

        DependencyKey::Path(path) => ("Path", None, vec![KeyOrPath::Path(path.to_owned())]),

        DependencyKey::DummyForTesting(inner_dep_key) => (
            "DummyForTesting",
            None,
            vec![KeyOrPath::Key(inner_dep_key.as_ref().clone())],
        ),
    };

    let mut buf = String::new();
    write!(&mut buf, "DependencyKeyV{VERSION}::{kind}(")?;
    if let Some(label) = maybe_label {
        write!(&mut buf, "{label}, ")?;
    }
    let tasks = values_to_hash
        .into_iter()
        .map(|key_or_hash| {
            let ctx = ctx.clone();
            match key_or_hash {
                KeyOrPath::Key(dep_key) => content_hash_dependency_key(ctx, dep_key),
                KeyOrPath::Path(path) => async move { content_hash_tree_path(&ctx, &path) }
                    .boxed()
                    .fuse()
                    .shared(),
            }
        })
        .map(|task| {
            ctx.thread_pool
                .spawn_with_handle(task)
                .map_err(|err| Error::SpawnTask(err.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let hashes = try_join_all(tasks).await?;
    for hash in hashes {
        write!(&mut buf, "{hash}, ")?;
    }
    write!(&mut buf, ")")?;
    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())
        .map_err(Error::HashObject)?;
    let hash = ContentHash(hash);
    Ok(hash)
}

/// Get the dependencies induced by the special
/// `tools/build_rules/prelude_bazel` file (if present). See
/// https://github.com/bazelbuild/bazel/issues/1674 for discussion on what this
/// file is.
pub fn get_prelude_deps(ctx: &HashContext) -> Result<BTreeSet<Label>> {
    if let Some(prelude_deps) = &ctx.caches.lock().unwrap().prelude_deps_cache {
        return Ok(prelude_deps.clone());
    }

    let prelude_dir = ["tools", "build_rules"];
    let prelude_file_name = "prelude_bazel";
    let prelude_path: PathBuf = prelude_dir.into_iter().chain([prelude_file_name]).collect();

    let result = {
        // TODO: don't leak repo
        let repo_state = ctx.repo_pool.create()?;
        match repo_state.borrow_head_tree().get_path(&prelude_path) {
            Ok(tree_entry) => {
                let mut result = BTreeSet::new();
                result.insert(Label {
                    external_repository: None,
                    path_components: prelude_dir.into_iter().map(|s| s.to_string()).collect(),
                    target_name: TargetName::Name(prelude_file_name.to_string()),
                });
                result.extend(extract_load_statements_from_tree_entry(
                    &repo_state,
                    &tree_entry,
                )?);
                result
            }
            Err(err) if err.code() == git2::ErrorCode::NotFound => Default::default(),
            Err(err) => return Err(Error::ReadTreeEntry(err)),
        }
    };

    ctx.caches.lock().unwrap().prelude_deps_cache = Some(result.clone());
    Ok(result)
}

fn content_hash_tree_path(ctx: &HashContext, path: &Path) -> Result<ContentHash> {
    if let Some(hash) = ctx.caches.lock().unwrap().tree_path_cache.get(path) {
        return Ok(hash.clone());
    }

    let mut buf = String::new();
    let tree_id = {
        // TODO: don't leak repo
        let repo_state = ctx.repo_pool.create()?;
        get_tree_path_id(repo_state.borrow_head_tree(), path).map_err(Error::ReadTreeEntry)?
    };
    write!(&mut buf, "PathBufV{VERSION}({tree_id})")?;

    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())
        .map_err(Error::HashObject)?;
    let hash = ContentHash(hash);
    if let Some(old_value) = ctx
        .caches
        .lock()
        .unwrap()
        .tree_path_cache
        .insert(path.to_owned(), hash.clone())
    {
        if old_value != hash {
            error!(key = ?path, ?old_value, new_value = ?hash, "Non-deterministic content hashing for path");
        }
    }
    Ok(hash)
}

fn get_tree_path_id(tree: &git2::Tree, path: &Path) -> std::result::Result<git2::Oid, git2::Error> {
    if path == Path::new("") {
        // `get_path` will produce an error if we pass an empty path, so
        // manually handle that here.
        Ok(tree.id())
    } else {
        match tree.get_path(path) {
            Ok(entry) => Ok(entry.id()),
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                // TODO: test this code path
                Ok(git2::Oid::zero())
            }
            Err(err) => Err(err),
        }
    }
}

fn get_tree_for_path<'repo>(
    repo_state: &'repo RepoState,
    package_path: &Path,
) -> Result<Option<git2::Tree<'repo>>> {
    if package_path == Path::new("") {
        Ok(Some(repo_state.borrow_head_tree().to_owned()))
    } else {
        let tree_entry = match repo_state.borrow_head_tree().get_path(package_path) {
            Ok(tree_entry) => tree_entry,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(Error::ReadTreeEntry(e)),
        };
        let object = tree_entry
            .to_object(repo_state.borrow_repo())
            .map_err(Error::ReadTreeEntry)?;
        let tree = object.as_tree().map(|tree| tree.to_owned());
        Ok(tree)
    }
}

fn find_load_dependencies(ctx: &HashContext, tree: &git2::Tree) -> Result<BTreeSet<Label>> {
    trace!(?tree, "Finding load dependencies");
    if let Some(result) = ctx
        .caches
        .lock()
        .unwrap()
        .load_dependencies_cache
        .get(&tree.id())
    {
        return Ok(result.clone());
    }

    let result = {
        // TODO: don't leak repo
        let repo_state = ctx.repo_pool.create()?;
        let mut result = BTreeSet::new();
        for tree_entry in tree {
            if is_tree_entry_relevant_to_build_graph(&tree_entry) {
                let deps = extract_load_statements_from_tree_entry(&repo_state, &tree_entry)?;
                result.extend(deps);
            }
        }
        result
    };

    if let Some(old_value) = ctx
        .caches
        .lock()
        .unwrap()
        .load_dependencies_cache
        .insert(tree.id(), result.clone())
    {
        if old_value != result {
            error!(key = ?tree.id(), ?old_value, new_value = ?result, "Non-deterministic content hashing for load dependencies");
        }
    }
    Ok(result)
}

fn is_tree_entry_relevant_to_build_graph(tree_entry: &git2::TreeEntry) -> bool {
    match tree_entry.name() {
        Some(file_name) => is_relevant_to_build_graph(Path::new(file_name)),
        None => {
            warn!(name_bytes = ?tree_entry.name_bytes(), "Skipped tree entry with non-UTF-8 name");
            false
        }
    }
}

fn extract_load_statements_from_tree_entry(
    repo_state: &RepoState,
    tree_entry: &git2::TreeEntry,
) -> Result<BTreeSet<Label>> {
    let object = tree_entry
        .to_object(repo_state.borrow_repo())
        .map_err(Error::ReadTreeEntry)?;
    let blob = match object.as_blob() {
        Some(blob) => blob,
        None => {
            warn!(file_name = ?tree_entry.name(), "Tree entry was not a blob");
            return Ok(Default::default());
        }
    };

    let content = match std::str::from_utf8(blob.content()) {
        Ok(content) => content,
        Err(e) => {
            warn!(file_name = ?tree_entry.name(), ?e, "Could not decode non-UTF-8 blob content");
            return Ok(Default::default());
        }
    };

    let deps = extract_load_statement_package_dependencies(content);
    Ok(deps)
}

fn extract_load_statement_package_dependencies(content: &str) -> BTreeSet<Label> {
    lazy_static! {
        static ref RE: Regex = Regex::new(
            r#"(?x)
# Literal "load".
load
\s*?

# Open parenthesis.
\(
\s*?

# String literal enclosed in quotes.
(?:
    "( [[:print:]--"]*? )"
  | '( [[:print:]--']*? )'
)

# Either a closing parenthesis or a comma to start the argument list.
\s*?
(?:
  ,
| \)
)
"#
        )
        .unwrap();
    }

    let mut result = BTreeSet::new();
    for cap in RE.captures_iter(content) {
        let value = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        let label: Label = match value.parse() {
            Ok(label) => label,
            Err(e) => {
                warn!(?e, "Failed to parse label in load statement");
                continue;
            }
        };
        result.insert(label);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_load_statements() -> Result<()> {
        let content = r#"
load("//foo/bar:baz.bzl")
load   (
    '//foo/qux:qux.bzl'

,    qux = 'grault')
"#;
        let labels = extract_load_statement_package_dependencies(content);
        insta::assert_debug_snapshot!(labels, @r###"
        {
            Label("//foo/bar:baz.bzl"),
            Label("//foo/qux:qux.bzl"),
        }
        "###);

        Ok(())
    }
}
