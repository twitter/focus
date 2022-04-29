use crate::{Cache, CompositeKey};
use anyhow::{Context, Result};
use core::fmt;
use focus_util::{app::App, git_helper::fetch_ref, git_helper::push_ref};
use git2::Oid;
use git2::{Commit, Repository};
use std::{collections::HashSet, path::PathBuf, str::FromStr, sync::Arc};
use tracing::{instrument, warn};

const COMMIT_USER_NAME: &str = "focus";
const COMMIT_USER_EMAIL: &str = "source-eng-team@twitter.com";

pub fn refspec_fmt(oid: Oid) -> String {
    return format!("+refs/tags/focus/{}:refs/tags/focus/{}", oid, oid);
}

pub fn tag_fmt(oid: Oid) -> String {
    return format!("refs/tags/focus/{}", oid);
}

pub trait CacheSynchronizer {
    fn fetch(&self, keyset_id: Oid) -> Result<()>;
    fn populate(&self, keyset_id: Oid, dest_cache: &dyn Cache) -> Result<()>;
    fn get_and_populate(&self, keyset_id: Oid, dest_cache: &dyn Cache) -> Result<()>;
    fn share(
        &self,
        keyset_id: Oid,
        keyset: &HashSet<(Oid, Oid)>,
        cache: &dyn Cache,
        previous_keyset_id: Option<Oid>,
    ) -> Result<()>;
}

pub struct GitBackedCacheSynchronizer {
    repo: Repository,
    app: Arc<App>,
    remote: String,
    path: PathBuf,
}

impl fmt::Debug for GitBackedCacheSynchronizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitBackedCacheSynchronizer")
            .field("remote", &self.remote)
            .field("path", &self.path.to_str())
            .finish()
    }
}

impl GitBackedCacheSynchronizer {
    pub fn create(path: PathBuf, remote: String, app: Arc<App>) -> Result<Self> {
        let repo = Repository::init(path.clone())?;
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
    fn fetch(&self, keyset_id: Oid) -> Result<()> {
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
    fn populate(&self, keyset_id: Oid, dest_cache: &dyn Cache) -> Result<()> {
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
            let composite_key = match CompositeKey::from_str(name) {
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
            match dest_cache.put(
                composite_key.function_id,
                composite_key.argument,
                blob.content(),
            ) {
                Ok(_) => continue,
                Err(_) => {
                    warn!(?object, "Failed to insert key into Cache");
                    continue;
                }
            };
        }
        Ok(())
    }

    fn get_and_populate(&self, keyset_id: Oid, dest_cache: &dyn Cache) -> Result<()> {
        self.fetch(keyset_id)?;
        self.populate(keyset_id, dest_cache)
    }

    #[instrument]
    fn share(
        &self,
        keyset_id: Oid,
        keyset: &HashSet<(Oid, Oid)>,
        cache: &dyn Cache,
        previous_keyset_id: Option<Oid>,
    ) -> Result<()> {
        let mut kv_tree = self
            .repo
            .treebuilder(None)
            .context("initializing new TreeBuilder")?;

        for (fn_id, arg_id) in keyset.iter() {
            let payload = cache.get(*fn_id, *arg_id)?.unwrap();
            let value_oid = self
                .repo
                .blob(&payload)
                .context("writing DependencyValue as blob")?;

            kv_tree
                .insert(
                    CompositeKey {
                        function_id: *fn_id,
                        argument: *arg_id,
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

    use crate::CacheSynchronizer;
    use crate::{tag_fmt, Cache, CompositeKey, RocksDBCache};
    use focus_util::app::App;

    const FN_ID: &str = "abcd1abcd1abcd1abcd1";
    const FN_ID2: &str = "abcd2abcd2abcd2abcd2";

    use super::GitBackedCacheSynchronizer;
    fn setup_server_repo_locally() -> Result<(TempDir, PathBuf)> {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join("server-repo");
        Repository::init_bare(file_path.clone()).context("failed to init")?;
        Ok((tmp_dir, file_path))
    }

    fn setup_local_sync_cache(
        name: &str,
        remote_repo: &str,
    ) -> (TempDir, Box<dyn CacheSynchronizer>) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join(name);
        let memocache = Box::new(
            GitBackedCacheSynchronizer::create(
                file_path,
                remote_repo.to_string(),
                Arc::new(App::new(false).unwrap()),
            )
            .unwrap(),
        );
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

    fn setup_rocks_db(name: &str) -> (TempDir, Box<dyn Cache>) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join(name);
        let cache = RocksDBCache::open(file_path.clone());
        (tmp_dir, Box::new(cache))
    }

    fn generate_random_key_values() -> (git2::Oid, Vec<u8>) {
        let mut rng = rand::thread_rng();
        let mut bytes: [u8; 20] = [0; 20];
        rng.fill(&mut bytes);
        let oid = Oid::from_bytes(&bytes[..]);
        let mut value: [u8; 1024] = [0; 1024];
        rng.fill(&mut value);
        (oid.unwrap(), value.to_vec())
    }

    fn populate_demo_hashset(memo_cache: &Box<dyn Cache>, fn_id: Oid) -> HashSet<(Oid, Oid)> {
        let mut pairs: HashSet<(Oid, Oid)> = HashSet::new();
        for _x in 1..100 {
            let (a, v) = generate_random_key_values();
            memo_cache.put(fn_id, a, &v[..]).unwrap();
            pairs.insert((fn_id, a));
        }
        pairs
    }

    fn assert_caches_match(
        commit_keys: HashSet<(Oid, Oid)>,
        cache1: &dyn Cache,
        cache2: &dyn Cache,
    ) {
        for (f, a) in commit_keys.iter() {
            assert_eq!(
                cache1.get(*f, *a).unwrap(),
                cache2.get(*f, *a).unwrap(),
                "key {}",
                CompositeKey {
                    function_id: *f,
                    argument: *a
                }
                .to_string()
            )
        }
    }

    fn assert_cache_doesnt_contain(commit_keys: HashSet<(Oid, Oid)>, cache1: &dyn Cache) {
        for (f, a) in commit_keys.iter() {
            assert!(cache1.get(*f, *a).unwrap().is_none());
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

        let keyset_id = Oid::from_str(FN_ID).unwrap();
        let fn_id = Oid::from_str(FN_ID).unwrap();
        // Tests that only the specified keys are pushed up.
        populate_demo_hashset(&memo_cache_1, keyset_id);

        let commit_1_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id, &commit_1_keys, &*memo_cache_1, None)?;
        memo_cache_sync_2.fetch(keyset_id).unwrap();

        //Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id, &commit_2_keys, &*memo_cache_1, None)?;

        memo_cache_sync_2
            .get_and_populate(keyset_id, &*memo_cache_2)
            .unwrap();

        assert_caches_match(commit_2_keys, &*memo_cache_1, &*memo_cache_2);
        assert_cache_doesnt_contain(commit_1_keys, &*memo_cache_2);
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

        let fn_id = Oid::from_str(FN_ID).unwrap();
        let keyset_id1 = Oid::from_str(FN_ID).unwrap();
        let commit_1_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id1, &commit_1_keys, &*memo_cache_1, None)?;

        let keyset_id2 = Oid::from_str(FN_ID2).unwrap();
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id2, &commit_2_keys, &*memo_cache_1, Some(keyset_id1))?;

        memo_cache_sync_2
            .get_and_populate(keyset_id2, &*memo_cache_2)
            .unwrap();

        let commit = memo_cache_sync_2
            .repo
            .find_reference(&tag_fmt(keyset_id2)[..])
            .unwrap()
            .peel_to_commit()
            .unwrap();
        assert!(commit.parent_count() == 1);

        assert_caches_match(commit_2_keys, &*memo_cache_1, &*memo_cache_2);
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

        let keyset_id = Oid::from_str(FN_ID).unwrap();
        let fn_id = Oid::from_str(FN_ID).unwrap();
        // Tests that only the specified keys are pushed up.
        populate_demo_hashset(&memo_cache_1, keyset_id);

        let commit_1_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id, &commit_1_keys, &*memo_cache_1, None)?;
        memo_cache_sync_2.fetch(keyset_id).unwrap();

        //Test overwriting an index (force push+pull)
        let commit_2_keys = populate_demo_hashset(&memo_cache_1, fn_id);
        memo_cache_sync_1.share(keyset_id, &commit_2_keys, &*memo_cache_1, None)?;

        memo_cache_sync_2
            .get_and_populate(keyset_id, &*memo_cache_2)
            .unwrap();

        assert_cache_doesnt_contain(commit_1_keys, &*memo_cache_2);
        assert_caches_match(commit_2_keys, &*memo_cache_1, &*memo_cache_2);
        Ok(())
    }
}
