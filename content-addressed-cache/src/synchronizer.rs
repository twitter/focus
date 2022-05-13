use crate::{Cache, CacheKey, CacheKeyKind, CompositeKey};
use anyhow::{Context, Result};
use core::fmt;
use focus_util::{app::App, git_helper::fetch_ref, git_helper::push_ref};
use git2::Oid;
use git2::{Commit, Repository};
use std::fmt::Debug;
use std::{collections::HashSet, path::PathBuf, str::FromStr, sync::Arc};
use tracing::{instrument, warn};

pub type Keyset = HashSet<(CacheKeyKind, CacheKey)>;
pub type KeysetID = Oid;

const COMMIT_USER_NAME: &str = "focus";
const COMMIT_USER_EMAIL: &str = "source-eng-team@twitter.com";

pub fn refspec_fmt(ksid: KeysetID) -> String {
    return format!("+refs/tags/focus/{}:refs/tags/focus/{}", ksid, ksid);
}

pub fn tag_fmt(ksid: KeysetID) -> String {
    return format!("refs/tags/focus/{}", ksid);
}

/// A synchronization mechanism for [Cache]s, which syncs key-value pairs.
///
/// This is optimized for synchronization in a situation where keys are
/// naturally grouped into "keysets" and many keysets overlap. For example,
/// build artifacts might form a graph of key-value pairs; but the set of all
/// build artifacts for a certain commit would belong in a single keyset. Since
/// most commits don't change most build artifacts, many of the key-value pairs
/// in the next commit's keyset can be shared with the current commit's keyset.
pub trait CacheSynchronizer: Debug {
    fn fetch(&self, keyset_id: KeysetID) -> Result<()>;
    fn populate(&self, keyset_id: KeysetID, dest_cache: &dyn Cache) -> Result<()>;
    fn get_and_populate(&self, keyset_id: KeysetID, dest_cache: &dyn Cache) -> Result<()>;
    fn share(
        &self,
        keyset_id: KeysetID,
        keyset: &Keyset,
        cache: &dyn Cache,
        previous_keyset_id: Option<KeysetID>,
    ) -> Result<()>;
}

/// Synchronize using Git as a key-value store. Keysets are pushed as tags to
/// the remote server. Key-value pairs are stored as entries in a tree, where
/// the entry name is the hash of the key and the entry value is a blob
/// containing the value's contents.
pub struct GitBackedCacheSynchronizer {
    repo: Repository,
    app: Arc<App>,
    remote: String,
    path: PathBuf,
}

impl fmt::Debug for GitBackedCacheSynchronizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitBackedCacheSynchronizer")
            .field("app", &self.app)
            .field("remote", &self.remote)
            .field("path", &self.path)
            .finish()
    }
}

impl GitBackedCacheSynchronizer {
    pub fn create(path: PathBuf, remote: String, app: Arc<App>) -> Result<Self> {
        let repo = Repository::init(path.clone()).context("initializing Git repository")?;
        Ok(Self {
            repo,
            app,
            remote,
            path,
        })
    }
}

impl CacheSynchronizer for GitBackedCacheSynchronizer {
    #[instrument]
    fn fetch(&self, keyset_id: KeysetID) -> Result<()> {
        let refspecs = refspec_fmt(keyset_id);
        fetch_ref(
            self.path.as_path(),
            refspecs.as_str(),
            self.remote.as_str(),
            self.app.clone(),
            Some(1),
        )?;
        Ok(())
    }

    #[instrument]
    fn populate(&self, keyset_id: KeysetID, dest_cache: &dyn Cache) -> Result<()> {
        let reference = self
            .repo
            .find_reference(&tag_fmt(keyset_id)[..])
            .context("Failed to find commit locally for given keyset_id.")?;

        let kv_tree = reference
            .peel_to_tree()
            .context("peeling kv tree reference")?;

        for tree_entry in kv_tree.iter() {
            let object = match tree_entry.to_object(&self.repo) {
                Ok(object) => object,
                Err(_) => {
                    warn!(
                        "Could not find object for tree entry {:X?}",
                        tree_entry.name_bytes()
                    );
                    continue;
                }
            };
            let name = match tree_entry.name() {
                Some(name) => name,
                None => {
                    warn!("{:X?} not utf-8", tree_entry.name_bytes());
                    continue;
                }
            };
            let CompositeKey { kind, key } = match CompositeKey::from_str(name) {
                Ok(key) => key,
                Err(_) => {
                    warn!("Tree entry {} is not a composite_key", name);
                    continue;
                }
            };
            let blob = match object.as_blob() {
                Some(blob) => blob,
                None => {
                    warn!(?object, "Tree entry was not a blob");
                    continue;
                }
            };
            match dest_cache.put(kind, key, blob.content()) {
                Ok(_) => continue,
                Err(_) => {
                    warn!(?object, "Failed to insert key into Cache");
                    continue;
                }
            };
        }
        Ok(())
    }

    fn get_and_populate(&self, keyset_id: KeysetID, dest_cache: &dyn Cache) -> Result<()> {
        self.fetch(keyset_id)?;
        self.populate(keyset_id, dest_cache)
    }

    #[instrument(skip(keyset))]
    fn share(
        &self,
        keyset_id: KeysetID,
        keyset: &Keyset,
        cache: &dyn Cache,
        previous_keyset_id: Option<KeysetID>,
    ) -> Result<()> {
        let mut kv_tree = self
            .repo
            .treebuilder(None)
            .context("initializing new TreeBuilder")?;

        for (kind, key) in keyset.iter() {
            let payload = cache.get(*kind, *key)?.unwrap();
            let value_oid = self
                .repo
                .blob(&payload)
                .context("writing DependencyValue as blob")?;

            kv_tree
                .insert(
                    CompositeKey {
                        key: *key,
                        kind: *kind,
                    }
                    .to_string(),
                    value_oid,
                    git2::FileMode::Blob.into(),
                )
                .context("adding entry to tree")?;
        }

        let kv_tree_oid = kv_tree.write().context("writing new tree")?;
        let signature = git2::Signature::now(COMMIT_USER_NAME, COMMIT_USER_EMAIL)?;
        let prev_commit_vec = match previous_keyset_id {
            Some(prev_commit_oid) => vec![self
                .repo
                .find_reference(&tag_fmt(prev_commit_oid)[..])?
                .peel_to_commit()?],
            None => vec![],
        };
        let vec_of_prev_commit_references: Vec<&Commit> = prev_commit_vec.iter().collect();

        let commit_oid = self.repo.commit(
            None,
            &signature,
            &signature,
            &format!("index for {}", keyset_id)[..],
            &self.repo.find_tree(kv_tree_oid)?,
            &vec_of_prev_commit_references[..],
        )?;
        self.repo
            .reference(
                &tag_fmt(keyset_id)[..],
                commit_oid,
                true,
                "Tree is used as key-value store.",
            )
            .context("updating reference")?;
        let refspecs = refspec_fmt(keyset_id);
        push_ref(
            self.path.as_path(),
            refspecs.as_str(),
            self.remote.as_str(),
            self.app.clone(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::{tempdir, TempDir};

    use git2::Oid;
    use rand::Rng;
    use std::collections::HashSet;

    use anyhow::{Context, Result};

    use crate::{tag_fmt, Cache, CompositeKey, RocksDBCache};
    use crate::{CacheKey, CacheKeyKind, CacheSynchronizer, Keyset};
    use focus_util::app::App;

    fn keyset_id_1() -> Oid {
        Oid::from_str("abcd1abcd1abcd1abcd1").unwrap()
    }

    fn keyset_id_2() -> Oid {
        Oid::from_str("abcd2abcd2abcd2abcd2").unwrap()
    }

    fn kind() -> [u8; 2] {
        hex::decode("f5b3").unwrap().try_into().unwrap()
    }

    use super::GitBackedCacheSynchronizer;
    fn setup_server_repo_locally() -> Result<(TempDir, PathBuf)> {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join("server-repo");
        Repository::init_bare(file_path.clone()).context("failed to init")?;
        Ok((tmp_dir, file_path))
    }

    fn setup_local_sync_cache(name: &str, remote_repo: &str) -> (TempDir, impl CacheSynchronizer) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join(name);
        let memocache = GitBackedCacheSynchronizer::create(
            file_path,
            remote_repo.to_string(),
            Arc::new(App::new(false).unwrap()),
        )
        .unwrap();
        (tmp_dir, memocache)
    }

    fn setup_local_git_cache(
        name: &str,
        remote_repo: &str,
    ) -> (TempDir, GitBackedCacheSynchronizer) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join(name);
        let memocache = GitBackedCacheSynchronizer::create(
            file_path,
            remote_repo.to_string(),
            Arc::new(App::new(false).unwrap()),
        )
        .unwrap();
        (tmp_dir, memocache)
    }

    fn setup_rocks_db(name: &str) -> (TempDir, impl Cache) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join(name);
        let cache = RocksDBCache::open(file_path);
        (tmp_dir, cache)
    }

    fn generate_random_key_values() -> (CacheKey, Vec<u8>) {
        let mut rng = rand::thread_rng();
        let mut bytes: [u8; 20] = [0; 20];
        rng.fill(&mut bytes);
        let ksid = Oid::from_bytes(&bytes[..]);
        let mut value: [u8; 1024] = [0; 1024];
        rng.fill(&mut value);
        (ksid.unwrap(), value.to_vec())
    }

    fn populate_demo_hashset(memo_cache: &dyn Cache, kind: CacheKeyKind) -> Keyset {
        let mut pairs: Keyset = HashSet::new();
        for _x in 1..100 {
            let (a, v) = generate_random_key_values();
            memo_cache.put(kind, a, &v[..]).unwrap();
            pairs.insert((kind, a));
        }
        pairs
    }

    fn assert_caches_match(commit_keys: Keyset, cache1: &dyn Cache, cache2: &dyn Cache) {
        for (kind, key) in commit_keys.iter() {
            assert_eq!(
                cache1.get(*kind, *key).unwrap(),
                cache2.get(*kind, *key).unwrap(),
                "key {}",
                CompositeKey {
                    kind: *kind,
                    key: *key
                }
                .to_string()
            )
        }
    }

    fn assert_cache_doesnt_contain(commit_keys: Keyset, cache1: &dyn Cache) {
        for (kind, key) in commit_keys.iter() {
            assert!(cache1.get(*kind, *key).unwrap().is_none());
        }
    }

    #[test]
    fn test_publish_get() -> anyhow::Result<()> {
        let (_server_dir, server_path) = setup_server_repo_locally().unwrap();
        let server_string = server_path.into_os_string().into_string().unwrap();
        let (_git_cache_dir_1, memo_cache_sync_1) =
            setup_local_sync_cache("fairly-local", server_string.as_str());
        let (_git_cache_dir_2, memo_cache_sync_2) =
            setup_local_sync_cache("fairly-local2", server_string.as_str());
        let (_rocks_dir_1, memo_cache_1) = setup_rocks_db("focus-rocks1");
        let (_rocks_dir_2, memo_cache_2) = setup_rocks_db("focus-rocks2");

        let keyset_id = keyset_id_1();
        let kind = kind();
        // Tests that only the specified keys are pushed up.
        populate_demo_hashset(&memo_cache_1, kind);

        let commit_1_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id, &commit_1_keys, &memo_cache_1, None)?;
        memo_cache_sync_2.fetch(keyset_id).unwrap();

        //Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id, &commit_2_keys, &memo_cache_1, None)?;

        memo_cache_sync_2
            .get_and_populate(keyset_id, &memo_cache_2)
            .unwrap();

        assert_caches_match(commit_2_keys, &memo_cache_1, &memo_cache_2);
        assert_cache_doesnt_contain(commit_1_keys, &memo_cache_2);
        Ok(())
    }

    #[test]
    fn test_previous_keyset_id() -> anyhow::Result<()> {
        let (_server_dir, server_path) = setup_server_repo_locally().unwrap();
        let server_string = server_path.into_os_string().into_string().unwrap();
        let (_git_cache_dir_1, memo_cache_sync_1) =
            setup_local_sync_cache("fairly-local", server_string.as_str());
        let (_git_cache_dir_2, memo_cache_sync_2) =
            setup_local_git_cache("fairly-local2", server_string.as_str());
        let (_rocks_dir_1, memo_cache_1) = setup_rocks_db("focus-rocks1");
        let (_rocks_dir_2, memo_cache_2) = setup_rocks_db("focus-rocks2");

        let kind = kind();
        let keyset_id1 = keyset_id_1();
        let commit_1_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id1, &commit_1_keys, &memo_cache_1, None)?;

        let keyset_id2 = keyset_id_2();
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id2, &commit_2_keys, &memo_cache_1, Some(keyset_id1))?;

        memo_cache_sync_2
            .get_and_populate(keyset_id2, &memo_cache_2)
            .unwrap();

        let commit = memo_cache_sync_2
            .repo
            .find_reference(&tag_fmt(keyset_id2)[..])
            .unwrap()
            .peel_to_commit()
            .unwrap();
        assert!(commit.parent_count() == 1);

        assert_caches_match(commit_2_keys, &memo_cache_1, &memo_cache_2);
        Ok(())
    }

    #[test]
    fn test_using_local_transport() -> anyhow::Result<()> {
        let (_server_dir, server_path) = setup_server_repo_locally().unwrap();
        let server_string = format!("file://{}", server_path.display());

        let (_git_cache_dir_1, memo_cache_sync_1) =
            setup_local_sync_cache("fairly-local", server_string.as_str());
        let (_git_cache_dir_2, memo_cache_sync_2) =
            setup_local_sync_cache("fairly-local2", server_string.as_str());
        let (_rocks_dir_1, memo_cache_1) = setup_rocks_db("focus-rocks1");
        let (_rocks_dir_2, memo_cache_2) = setup_rocks_db("focus-rocks2");

        let keyset_id = keyset_id_1();
        let kind = kind();
        // Tests that only the specified keys are pushed up.
        populate_demo_hashset(&memo_cache_1, kind);

        let commit_1_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id, &commit_1_keys, &memo_cache_1, None)?;
        memo_cache_sync_2.fetch(keyset_id).unwrap();

        //Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        memo_cache_sync_1.share(keyset_id, &commit_2_keys, &memo_cache_1, None)?;

        memo_cache_sync_2
            .get_and_populate(keyset_id, &memo_cache_2)
            .unwrap();

        assert_cache_doesnt_contain(commit_1_keys, &memo_cache_2);
        assert_caches_match(commit_2_keys, &memo_cache_1, &memo_cache_2);
        Ok(())
    }
}
