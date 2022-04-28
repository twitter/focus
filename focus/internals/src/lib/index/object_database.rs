use std::time::Duration;

use super::content_hash::HashContext;
use super::{content_hash_dependency_key, ContentHash, DependencyKey, DependencyValue};
use anyhow::Context;
use content_addressed_cache::Cache;
use lazy_static::lazy_static;
use tracing::{debug, info, warn};

pub use content_addressed_cache::RocksDBCache;

/// A persistent key-value cache mapping the hashes of [`super::DependencyKey`]s
/// to [`DependencyValue`]s.
pub trait ObjectDatabase {
    /// Look up the key-value pair with the provided key hash.
    fn get(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
    ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)>;

    /// Insert a new key-value pair into persistent storage.
    fn put(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
        value: DependencyValue,
    ) -> anyhow::Result<()>;

    /// Clear all entries.
    fn clear(&self) -> anyhow::Result<()>;
}

lazy_static! {
    static ref FUNCTION_ID: git2::Oid =
        git2::Oid::hash_object(git2::ObjectType::Blob, b"odb").unwrap();
}

impl<T: Cache> ObjectDatabase for T {
    fn get(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
    ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)> {
        let hash = content_hash_dependency_key(ctx, key)?;
        let result = match self.get(hash.0, *FUNCTION_ID)? {
            Some(content) => serde_json::from_slice(&content[..])
                .context("deserializing DependencyValue as JSON")?,
            None => None,
        };
        Ok((hash, result))
    }

    fn put(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
        value: DependencyValue,
    ) -> anyhow::Result<()> {
        let hash = content_hash_dependency_key(ctx, key)?;
        debug!(?hash, ?value, "Inserting entry into object database");
        let payload = serde_json::to_vec(&value).context("serializing DependencyValue as JSON")?;
        self.put(hash.0, *FUNCTION_ID, &payload[..])?;
        Ok(())
    }

    fn clear(&self) -> anyhow::Result<()> {
        self.clear()?;
        Ok(())
    }
}

/// Helper functions to use [`RocksDBMemoizationCache`] as an [`ObjectDatabase`].
pub trait RocksDBMemoizationCacheExt {
    /// Create the cache in a fixed directory under `.git`.
    fn new(repo: &git2::Repository) -> Self;
}

impl RocksDBMemoizationCacheExt for RocksDBCache {
    fn new(repo: &git2::Repository) -> RocksDBCache {
        let rocksdb_path = repo.path().join("focus/focus-index-rocks-db");
        RocksDBCache::open_with_ttl(rocksdb_path, Duration::from_secs(3600 * 24 * 90))
    }
}

/// Testing utilities.
pub mod testing {
    use crate::index::content_hash_dependency_key;

    use super::*;

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tracing::{debug, error};

    /// Object database backed by an in-memory hashmap. This doesn't actually
    /// persist data between invocations of the program, so it's primarily
    /// useful for testing.
    #[derive(Clone, Debug, Default)]
    pub struct HashMapOdb {
        entries: Arc<Mutex<HashMap<ContentHash, DependencyValue>>>,
    }

    impl HashMapOdb {
        /// Constructor.
        pub fn new() -> Self {
            Self {
                entries: Default::default(),
            }
        }
    }

    impl ObjectDatabase for HashMapOdb {
        fn get(
            &self,
            ctx: &HashContext,
            key: &DependencyKey,
        ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)> {
            let hash = content_hash_dependency_key(ctx, key)?;
            let entries = self.entries.lock().expect("poisoned mutex");
            let dep_value = entries.get(&hash).cloned();
            Ok((hash, dep_value))
        }

        fn put(
            &self,
            ctx: &HashContext,
            key: &DependencyKey,
            value: DependencyValue,
        ) -> anyhow::Result<()> {
            let hash = content_hash_dependency_key(ctx, key)?;
            debug!(?hash, ?value, "Inserting entry into object database");

            let mut entries = self.entries.lock().expect("poisoned mutex");
            if let Some(old_value) = entries.insert(hash.clone(), value.clone()) {
                if value != old_value {
                    error!(
                        ?key,
                        ?old_value,
                        new_value = ?value,
                        ?hash,
                        "Non-deterministic dependency hashing"
                    );
                }
            }
            Ok(())
        }

        fn clear(&self) -> anyhow::Result<()> {
            self.entries.lock().unwrap().clear();
            Ok(())
        }
    }
}

/// Simple object database which stores key-value pairs in the same repository
/// that's being worked in.
#[derive(Clone)]
pub struct SimpleGitOdb<'a> {
    repo: &'a git2::Repository,
}

impl<'a> SimpleGitOdb<'a> {
    const REF_NAME: &'static str = "refs/focus/simple_kv_tree";

    /// Constructor.
    pub fn new(repo: &'a git2::Repository) -> SimpleGitOdb<'a> {
        Self { repo }
    }
}

impl ObjectDatabase for SimpleGitOdb<'_> {
    fn get(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
    ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)> {
        let hash @ ContentHash(key_oid) = content_hash_dependency_key(ctx, key)?;

        let kv_tree = match ctx.repo.find_reference(Self::REF_NAME) {
            Ok(reference) => reference
                .peel_to_tree()
                .context("peeling kv tree reference")?,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok((hash, None)),
            Err(e) => return Err(e.into()),
        };

        let tree_entry = match kv_tree.get_name(&key_oid.to_string()) {
            Some(tree_entry) => tree_entry,
            None => return Ok((hash, None)),
        };

        let object = match tree_entry.to_object(ctx.repo) {
            Ok(object) => object,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok((hash, None)),
            Err(e) => return Err(e.into()),
        };
        let blob = match object.as_blob() {
            Some(blob) => blob,
            None => {
                warn!(?object, "Tree entry was not a blob");
                return Ok((hash, None));
            }
        };

        let content = blob.content();
        let result: DependencyValue =
            serde_json::from_slice(content).context("deserializing DependencyValue as JSON")?;
        Ok((hash, Some(result)))
    }

    fn put(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
        value: DependencyValue,
    ) -> anyhow::Result<()> {
        let ContentHash(key_oid) = content_hash_dependency_key(ctx, key)?;
        let payload = serde_json::to_vec(&value).context("serializing DependencyValue as JSON")?;
        let value_oid = ctx
            .repo
            .blob(&payload)
            .context("writing DependencyValue as blob")?;

        let mut kv_tree = match ctx.repo.find_reference(Self::REF_NAME) {
            Ok(reference) => {
                let tree = reference
                    .peel_to_tree()
                    .context("peeling kv tree reference")?;
                ctx.repo
                    .treebuilder(Some(&tree))
                    .context("initializing TreeBuilder from kv tree reference")?
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => ctx
                .repo
                .treebuilder(None)
                .context("initializing new TreeBuilder")?,
            Err(e) => return Err(e.into()),
        };
        kv_tree
            .insert(key_oid.to_string(), value_oid, git2::FileMode::Blob.into())
            .context("adding entry to tree")?;
        let kv_tree_oid = kv_tree.write().context("writing new tree")?;
        ctx.repo
            .reference(
                Self::REF_NAME,
                kv_tree_oid,
                true,
                &format!("updating with key {:?}", key),
            )
            .context("updating reference")?;
        Ok(())
    }

    fn clear(&self) -> anyhow::Result<()> {
        match self.repo.find_reference(Self::REF_NAME) {
            Ok(mut reference) => {
                info!(
                    reference_name = reference.name(),
                    "Deleting SimpleGitOdb reference"
                );
                reference.delete().context("deleting reference")?;
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                // Do nothing.
                info!(
                    reference_name = Self::REF_NAME,
                    "No SimpleGitOdb reference to delete; nothing to do"
                );
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreeset;

    use focus_testing::scratch_git_repo::ScratchGitRepo;

    use crate::coordinate::{Label, TargetName};

    use super::*;

    #[test]
    fn test_simple_git_odb() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp_dir.path())?;
        let repo = fix.repo()?;
        let odb = SimpleGitOdb::new(&repo);

        let head_tree_oid = repo.treebuilder(None)?.write()?;
        let head_tree = repo.find_tree(head_tree_oid)?;
        let ctx = HashContext {
            repo: &repo,
            head_tree: &head_tree,
            caches: Default::default(),
        };
        let key = DependencyKey::BazelPackage(Label {
            external_repository: None,
            path_components: vec!["foo".to_string(), "bar".to_string()],
            target_name: TargetName::Name("bar".to_string()),
        });
        let value = DependencyValue::PackageInfo {
            deps: btreeset! {
                DependencyKey::BazelPackage(Label {
                    external_repository: None,
                    path_components: vec!["baz".to_string(), "qux".to_string()],
                    target_name: TargetName::Name("qux".to_string()),
                }),
            },
        };
        assert!(odb.get(&ctx, &key)?.1.is_none());

        odb.put(&ctx, &key, value.clone())?;
        assert_eq!(odb.get(&ctx, &key)?.1, Some(value));

        Ok(())
    }
}
