use crate::{Cache, CacheKey, CacheKeyKind, CompositeKey};
use anyhow::{Context, Result};
use core::fmt;
use focus_util::{app::App, git_helper};
use git2::Oid;
use git2::{Commit, Repository};
use std::fmt::{Debug, Display};
use std::ops::AddAssign;
use std::{collections::HashSet, path::PathBuf, str::FromStr, sync::Arc};
use tracing::{info, instrument, warn};

pub type Keyset = HashSet<(CacheKeyKind, CacheKey)>;
pub type KeysetID = Oid;

const COMMIT_USER_NAME: &str = "focus";
const COMMIT_USER_EMAIL: &str = "source-eng-team@twitter.com";

pub fn refspec_fmt(value: impl Display) -> String {
    return format!("+refs/tags/focus/{}:refs/tags/focus/{}", value, value);
}

pub fn tag_fmt(value: impl Display) -> String {
    return format!("refs/tags/focus/{}", value);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PopulateResult {
    pub entry_count: usize,
    pub new_entry_count: usize,
    pub failed_entry_count: usize,
}

impl AddAssign for PopulateResult {
    fn add_assign(&mut self, rhs: Self) {
        let Self {
            entry_count,
            new_entry_count,
            failed_entry_count,
        } = rhs;
        self.entry_count += entry_count;
        self.new_entry_count += new_entry_count;
        self.failed_entry_count += failed_entry_count;
    }
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
    fn fetch(&self) -> Result<HashSet<git2::Oid>>;
    fn populate(&self, commit_id: &git2::Oid, dest_cache: &dyn Cache) -> Result<PopulateResult>;
    fn fetch_and_populate(
        &self,
        dest_cache: &dyn Cache,
    ) -> Result<(PopulateResult, HashSet<git2::Oid>)>;
    fn share(
        &self,
        keyset_id: KeysetID,
        keyset: &Keyset,
        cache: &dyn Cache,
        previous_keyset_id: Option<KeysetID>,
        additional_ref_name: Option<&str>,
    ) -> Result<git2::Oid>;
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
    fn fetch(&self) -> Result<HashSet<git2::Oid>> {
        git_helper::fetch_all_tags(
            self.path.as_path(),
            self.remote.as_str(),
            self.app.clone(),
            Some(1),
        )
        .context("Fetching")
    }

    #[instrument]
    fn populate(&self, commit_id: &git2::Oid, dest_cache: &dyn Cache) -> Result<PopulateResult> {
        let commit = self
            .repo
            .find_commit(*commit_id)
            .context("Resolving commit")?;
        let kv_tree = commit.tree().context("Resolving tree")?;

        let mut entry_count = 0;
        let mut new_entry_count = 0;
        let mut failed_entry_count = 0;
        for tree_entry in kv_tree.iter() {
            entry_count += 1;

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

            match dest_cache.get(kind, key) {
                Ok(Some(_)) => {
                    // Already present, do nothing.
                    continue;
                }
                Ok(None) => {
                    // Insert below.
                }
                Err(e) => {
                    // Insert below.
                    warn!(?kind, ?key, ?e, "Failed to get key from cache");
                }
            }

            match dest_cache.put(kind, key, blob.content()) {
                Ok(()) => {
                    new_entry_count += 1;
                }
                Err(e) => {
                    failed_entry_count += 1;
                    warn!(?object, ?e, "Failed to insert key into Cache");
                }
            }
        }

        Ok(PopulateResult {
            entry_count,
            new_entry_count,
            failed_entry_count,
        })
    }

    fn fetch_and_populate(
        &self,
        dest_cache: &dyn Cache,
    ) -> Result<(PopulateResult, HashSet<git2::Oid>)> {
        let fetched_commits = self.fetch().context("Fetching index updates")?;
        let mut result = PopulateResult::default();
        for commit_id in fetched_commits.iter() {
            let commit_id_str = commit_id.to_string();
            let populate_result = self
                .populate(commit_id, dest_cache)
                .with_context(|| format!("Populating cache from commit {}", &commit_id_str))?;
            info!(?populate_result, commit_id = %commit_id_str, "Populated index");
            result += populate_result;
        }
        Ok((result, fetched_commits))
    }

    #[instrument(skip(keyset))]
    fn share(
        &self,
        keyset_id: KeysetID,
        keyset: &Keyset,
        cache: &dyn Cache,
        previous_keyset_id: Option<KeysetID>,
        additional_ref_name: Option<&str>,
    ) -> Result<git2::Oid> {
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

        let mut refspecs = vec![refspec_fmt(&keyset_id)];
        if let Some(additional_ref_name) = additional_ref_name {
            refspecs.push(refspec_fmt(&additional_ref_name))
        }

        self.repo
            .reference(
                &tag_fmt(keyset_id)[..],
                commit_oid,
                true,
                "Tree is used as key-value store.",
            )
            .context("updating reference")?;
        if let Some(ref_name) = additional_ref_name {
            self.repo
                .reference(
                    &tag_fmt(ref_name),
                    commit_oid,
                    true,
                    "Tree is used as key-value store.",
                )
                .context("updating additional reference")?;
        }
        git_helper::push_refs(
            self.path.as_path(),
            refspecs.into_iter(),
            self.remote.as_str(),
            self.app.clone(),
        )?;
        Ok(commit_oid)
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

    use maplit::hashset;

    use super::*;

    const RANDOM_KEY_COUNT: usize = 100;

    fn keyset_id_1() -> Oid {
        Oid::from_str("abcd1abcd1abcd1abcd1").unwrap()
    }

    fn keyset_id_2() -> Oid {
        Oid::from_str("abcd2abcd2abcd2abcd2").unwrap()
    }

    fn kind() -> [u8; 2] {
        hex::decode("f5b3").unwrap().try_into().unwrap()
    }

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
        for _x in 0..RANDOM_KEY_COUNT {
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
        let commit_1_id =
            memo_cache_sync_1.share(keyset_id, &commit_1_keys, &memo_cache_1, None, None)?;
        let fetched_commit_ids = memo_cache_sync_2.fetch().unwrap();
        assert_eq!(fetched_commit_ids, hashset! { commit_1_id });

        // Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        let commit_2_id =
            memo_cache_sync_1.share(keyset_id, &commit_2_keys, &memo_cache_1, None, None)?;

        let (_, fetched_commit_ids) = memo_cache_sync_2.fetch_and_populate(&memo_cache_2).unwrap();
        assert_eq!(fetched_commit_ids, hashset! { commit_2_id });

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
        let commit_1_id =
            memo_cache_sync_1.share(keyset_id1, &commit_1_keys, &memo_cache_1, None, None)?;

        let keyset_id2 = keyset_id_2();
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        let commit_2_id = memo_cache_sync_1.share(
            keyset_id2,
            &commit_2_keys,
            &memo_cache_1,
            Some(keyset_id1),
            None,
        )?;

        let (_, fetched_commit_ids) = memo_cache_sync_2.fetch_and_populate(&memo_cache_2).unwrap();

        assert_eq!(fetched_commit_ids, hashset! {commit_1_id, commit_2_id});

        let commit = memo_cache_sync_2.repo.find_commit(commit_2_id).unwrap();
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
        let commit_1_id =
            memo_cache_sync_1.share(keyset_id, &commit_1_keys, &memo_cache_1, None, None)?;
        let fetched_commit_ids = memo_cache_sync_2.fetch().unwrap();
        assert_eq!(fetched_commit_ids, hashset! { commit_1_id  });

        // Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, kind);
        let commit_2_id =
            memo_cache_sync_1.share(keyset_id, &commit_2_keys, &memo_cache_1, None, None)?;

        let (results, fetched_commit_ids) =
            memo_cache_sync_2.fetch_and_populate(&memo_cache_2).unwrap();
        assert_eq!(fetched_commit_ids, hashset! { commit_2_id  });
        assert_eq!(
            results,
            PopulateResult {
                new_entry_count: RANDOM_KEY_COUNT,
                entry_count: RANDOM_KEY_COUNT,
                failed_entry_count: 0
            }
        );

        assert_cache_doesnt_contain(commit_1_keys, &memo_cache_2);
        assert_caches_match(commit_2_keys, &memo_cache_1, &memo_cache_2);
        Ok(())
    }

    #[test]
    pub fn refspec_formatting() {
        assert_eq!(refspec_fmt(&keyset_id_1()), String::from("+refs/tags/focus/abcd1abcd1abcd1abcd100000000000000000000:refs/tags/focus/abcd1abcd1abcd1abcd100000000000000000000"));
        assert_eq!(
            refspec_fmt("foo"),
            String::from("+refs/tags/focus/foo:refs/tags/focus/foo")
        );
    }

    #[test]
    pub fn tag_formatting() {
        assert_eq!(
            tag_fmt(&keyset_id_1()),
            String::from("refs/tags/focus/abcd1abcd1abcd1abcd100000000000000000000"),
        );
        assert_eq!(tag_fmt("foo"), String::from("refs/tags/focus/foo"));
    }
}
