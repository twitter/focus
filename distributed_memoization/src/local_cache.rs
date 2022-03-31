use std::default::Default;
use std::string::ToString;
use std::time::Duration;

use anyhow::{self, Context};
use git2::Oid;
use rocksdb::{Options, DB};
use std::path::PathBuf;
use tracing::warn;

pub trait MemoizationCache {
    fn insert(&self, function_id: Oid, argument: Oid, value: &[u8]) -> anyhow::Result<()>;
    fn get(&self, function_id: Oid, argument: Oid) -> anyhow::Result<Option<Vec<u8>>>;
    fn clear(&self) -> anyhow::Result<()>;
}

pub struct RocksDBMemoizationCache {
    db: DB,
}

pub struct CompositeKey {
    argument: Oid,
    function_id: Oid,
}

#[derive(Debug, Clone)]
pub struct ParseCompositeKeyError;

impl ToString for CompositeKey {
    fn to_string(&self) -> String {
        format!("{},{}", self.argument, self.function_id)
    }
}

const KEY_PREFIX_LENGTH: usize = 3;
const KEY_PREFIX: &[u8; KEY_PREFIX_LENGTH] = b"oid";
const OID_BYTE_LENGTH: usize = 20;
const COMPOSITE_KEY_LENGTH: usize = KEY_PREFIX_LENGTH + OID_BYTE_LENGTH + OID_BYTE_LENGTH;
type CompositeKeyBytes = [u8; COMPOSITE_KEY_LENGTH];

impl CompositeKey {
    pub fn to_bytes(&self) -> CompositeKeyBytes {
        let mut c: [u8; COMPOSITE_KEY_LENGTH] = [0; COMPOSITE_KEY_LENGTH];
        c[..KEY_PREFIX_LENGTH].clone_from_slice(KEY_PREFIX);
        c[KEY_PREFIX_LENGTH..KEY_PREFIX_LENGTH + OID_BYTE_LENGTH]
            .clone_from_slice(self.function_id.as_bytes());
        c[KEY_PREFIX_LENGTH + OID_BYTE_LENGTH..COMPOSITE_KEY_LENGTH]
            .clone_from_slice(self.argument.as_bytes());
        c
    }

    pub fn from_bytes(s: &[u8; COMPOSITE_KEY_LENGTH]) -> Result<Self, ParseCompositeKeyError> {
        if !s.starts_with(KEY_PREFIX) {
            return Err(ParseCompositeKeyError);
        }
        let function_id =
            match Oid::from_bytes(&s[KEY_PREFIX_LENGTH..KEY_PREFIX_LENGTH + OID_BYTE_LENGTH]) {
                Ok(oid) => oid,
                Err(_) => return Err(ParseCompositeKeyError),
            };
        let argument =
            match Oid::from_bytes(&s[KEY_PREFIX_LENGTH + OID_BYTE_LENGTH..COMPOSITE_KEY_LENGTH]) {
                Ok(oid) => oid,
                Err(_) => return Err(ParseCompositeKeyError),
            };
        Ok(CompositeKey {
            function_id,
            argument,
        })
    }
}

impl RocksDBMemoizationCache {
    pub fn open_with_ttl(path: PathBuf, ttl: Duration) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        Self {
            db: DB::open_with_ttl(&opts, path, ttl).unwrap(),
        }
    }
    pub fn open(path: PathBuf) -> Self {
        Self::open_with_ttl(path, Duration::from_secs(0))
    }
}

impl MemoizationCache for RocksDBMemoizationCache {
    fn insert(&self, function_id: Oid, argument: Oid, value: &[u8]) -> anyhow::Result<()> {
        let key: &[u8] = &CompositeKey {
            function_id,
            argument,
        }
        .to_bytes()[..];
        self.db
            .put(key, value)
            .with_context(|| format!("Putting {:?} failed", key))
    }

    fn get(&self, function_id: Oid, argument: Oid) -> anyhow::Result<Option<Vec<u8>>> {
        let key: &[u8] = &CompositeKey {
            function_id,
            argument,
        }
        .to_bytes()[..];
        self.db
            .get(key)
            .with_context(|| format!("Getting {:?} failed", key))
    }
    fn clear(&self) -> anyhow::Result<()> {
        // FIXME: maybe use `DB::destroy`?
        warn!("clear not yet implemented for RocksDB backend");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use git2::Oid;
    use rocksdb::{Options, DB};
    use tempfile::{tempdir, TempDir};

    use crate::{
        local_cache::OID_BYTE_LENGTH, CompositeKey, MemoizationCache, RocksDBMemoizationCache,
    };

    static ARG: &'static str = "12345678912345789ab";
    static FN_ID: &'static str = "abcd5abcd5abcd5abcd5";
    static BAD_OID: &'static str = "deadbeefdeadbeefdead";

    #[test]
    fn test_rocks_ttl_0() {
        let mut opts = Options::default();
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join("focus-rocks");
        opts.create_if_missing(true);
        let db = DB::open_with_ttl(&opts, file_path, Duration::from_secs(0)).unwrap();
        db.put("abcd", "abcd").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        db.compact_range(Some("aacd"), Some("accd"));
        assert_eq!(db.get("abcd").unwrap().unwrap(), b"abcd".to_vec());
    }

    #[test]
    fn test_rocks_ttl_1() {
        let mut opts = Options::default();
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join("focus-rocks");
        opts.create_if_missing(true);
        let db = DB::open_with_ttl(&opts, file_path, Duration::from_secs(1)).unwrap();
        db.put("abcd", "abcd").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        db.compact_range(Some("aacd"), Some("accd"));
        assert_eq!(db.get("abcd").unwrap(), None);
    }

    fn create_test_repo() -> (TempDir, PathBuf) {
        let tmp_dir = tempdir().unwrap();
        let file_path = tmp_dir.path().join("focus-rocks");
        {
            let cache = RocksDBMemoizationCache::open(file_path.clone());
            cache
                .insert(
                    Oid::from_str(FN_ID).unwrap(),
                    Oid::from_str(ARG).unwrap(),
                    b"abcd",
                )
                .unwrap();
        }
        (tmp_dir, file_path)
    }

    #[test]
    fn test_key_insert_get() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBMemoizationCache::open(file_path);
        std::thread::sleep(Duration::from_secs(2));
        let value = cache.get(Oid::from_str(FN_ID).unwrap(), Oid::from_str(ARG).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_key_insert_get_ttl() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBMemoizationCache::open_with_ttl(file_path, Duration::from_secs(3600));
        let value = cache.get(Oid::from_str(FN_ID).unwrap(), Oid::from_str(ARG).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_arg_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBMemoizationCache::open(file_path);
        let value = cache.get(
            Oid::from_str(FN_ID).unwrap(),
            Oid::from_str(BAD_OID).unwrap(),
        );
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_function_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBMemoizationCache::open(file_path);
        let value = cache.get(Oid::from_str(BAD_OID).unwrap(), Oid::from_str(ARG).unwrap());
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_compositekey() {
        let oid_bytes = CompositeKey {
            argument: Oid::from_str(ARG).unwrap(),
            function_id: Oid::from_str(FN_ID).unwrap(),
        }
        .to_bytes();
        let inflated_oid = CompositeKey::from_bytes(&oid_bytes).unwrap();
        assert_eq!(inflated_oid.argument, Oid::from_str(ARG).unwrap());
        assert_eq!(inflated_oid.function_id, Oid::from_str(FN_ID).unwrap());
    }

    #[test]
    fn test_oid_invariants() {
        assert_eq!(OID_BYTE_LENGTH, Oid::zero().as_bytes().len());
    }
}
