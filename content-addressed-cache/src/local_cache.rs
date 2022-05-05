use std::default::Default;
use std::string::ToString;
use std::time::Duration;
use std::{cell::RefCell, fmt::Debug, str::FromStr};

use anyhow::{self, Context};
use rocksdb::{Options, DB};
use std::path::{Path, PathBuf};

/// The "kind" of a key. This is combined with the [CacheKey] to produce a
/// composite key. It effectively introduces a namespace for a set of keys.
pub type CacheKeyKind = [u8; 2];

/// The key which is used to index into the cache.
pub type CacheKey = git2::Oid;

/// A persistent cache for key-value pairs. This is designed to be synchronized
/// with a remote store; see [crate::CacheSynchronizer].
pub trait Cache: Debug {
    fn put(&self, kind: CacheKeyKind, key: CacheKey, value: &[u8]) -> anyhow::Result<()>;
    fn get(&self, kind: CacheKeyKind, key: CacheKey) -> anyhow::Result<Option<Vec<u8>>>;
    fn clear(&self) -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct RocksDBCache {
    db: RefCell<Option<DB>>,
    ttl: Duration,
}

#[derive(Debug, PartialEq)]
pub struct CompositeKey {
    pub kind: CacheKeyKind,
    pub key: CacheKey,
}

#[derive(Debug, Clone)]
pub enum ParseCompositeKeyError {
    NoMatch,
    MissingPrefix,
    MissingFunctionIdentifier,
    MissingArgument,
    MissingDelimiter,
}

impl ToString for CompositeKey {
    fn to_string(&self) -> String {
        let CompositeKey { kind, key } = self;
        format!("{}{}{}", hex::encode(kind), DELIMITER, key)
    }
}

const KIND_BYTE_LENGTH: usize = 2;
const KEY_BYTE_LENGTH: usize = 20;
pub const DELIMITER: &str = ":";
const COMPOSITE_KEY_LENGTH: usize = KIND_BYTE_LENGTH + KEY_BYTE_LENGTH;
const HEX_ENCODED_COMPOSITE_KEY_LENGTH: usize =
    (KIND_BYTE_LENGTH * 2) + DELIMITER.len() + (KEY_BYTE_LENGTH * 2);
type CompositeKeyBytes = [u8; COMPOSITE_KEY_LENGTH];

impl CompositeKey {
    pub fn to_bytes(&self) -> CompositeKeyBytes {
        let mut c: [u8; COMPOSITE_KEY_LENGTH] = [0; COMPOSITE_KEY_LENGTH];
        c[..KIND_BYTE_LENGTH].clone_from_slice(&self.kind);
        c[KIND_BYTE_LENGTH..COMPOSITE_KEY_LENGTH].clone_from_slice(self.key.as_bytes());
        c
    }

    pub fn from_bytes(s: &[u8; COMPOSITE_KEY_LENGTH]) -> Result<Self, ParseCompositeKeyError> {
        let mut kind = [0; 2];
        kind.clone_from_slice(&s[..KIND_BYTE_LENGTH]);
        let key = match CacheKey::from_bytes(&s[KIND_BYTE_LENGTH..COMPOSITE_KEY_LENGTH]) {
            Ok(oid) => oid,
            Err(_) => return Err(ParseCompositeKeyError::MissingArgument),
        };
        Ok(CompositeKey { kind, key })
    }
}

impl FromStr for CompositeKey {
    type Err = ParseCompositeKeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ix: usize = 0;
        if s.len() != HEX_ENCODED_COMPOSITE_KEY_LENGTH {
            return Err(ParseCompositeKeyError::NoMatch);
        };
        let kind_vec = match hex::decode(&s[ix..ix + (KIND_BYTE_LENGTH * 2)]) {
            Ok(oid) => oid,
            Err(_) => return Err(ParseCompositeKeyError::MissingFunctionIdentifier),
        };
        ix += KIND_BYTE_LENGTH * 2;
        if &s[ix..ix + 1] != DELIMITER {
            return Err(ParseCompositeKeyError::MissingDelimiter);
        }
        ix += 1;
        let key = match CacheKey::from_str(&s[ix..ix + (KEY_BYTE_LENGTH * 2)]) {
            Ok(oid) => oid,
            Err(_) => return Err(ParseCompositeKeyError::MissingArgument),
        };
        let mut kind = [0; 2];
        kind.copy_from_slice(&kind_vec[..2]);
        Ok(CompositeKey { kind, key })
    }
}

impl RocksDBCache {
    fn make_db(path: &Path, ttl: Duration) -> DB {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        DB::open_with_ttl(&opts, path, ttl).unwrap()
    }

    pub fn open_with_ttl(path: impl AsRef<Path>, ttl: Duration) -> Self {
        Self {
            db: RefCell::new(Some(Self::make_db(path.as_ref(), ttl))),
            ttl,
        }
    }
    pub fn open(path: PathBuf) -> Self {
        Self::open_with_ttl(path, Duration::from_secs(0))
    }
}

impl Cache for RocksDBCache {
    fn put(&self, kind: CacheKeyKind, key: CacheKey, value: &[u8]) -> anyhow::Result<()> {
        let key: &[u8] = &CompositeKey { kind, key }.to_bytes()[..];
        self.db
            .borrow()
            .as_ref()
            .unwrap()
            .put(key, value)
            .with_context(|| format!("Putting {:?} failed", key))
    }

    fn get(&self, kind: CacheKeyKind, key: CacheKey) -> anyhow::Result<Option<Vec<u8>>> {
        let key: &[u8] = &CompositeKey { kind, key }.to_bytes()[..];
        self.db
            .borrow()
            .as_ref()
            .unwrap()
            .get(key)
            .with_context(|| format!("Getting {:?} failed", key))
    }

    fn clear(&self) -> anyhow::Result<()> {
        let path = self.db.borrow().as_ref().unwrap().path().to_path_buf();
        {
            let db = self.db.borrow_mut().take().unwrap();
            drop(db);
        }
        DB::destroy(&Options::default(), &path)?;
        *self.db.borrow_mut() = Some(Self::make_db(&path, self.ttl));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::{path::PathBuf, str::FromStr};

    use rocksdb::{Options, DB};
    use tempfile::{tempdir, TempDir};

    use crate::local_cache::DELIMITER;
    use crate::{local_cache::KEY_BYTE_LENGTH, Cache, CacheKey, CompositeKey, RocksDBCache};

    static KEY: &str = "12345678912345789ab";
    static HEX_KEY: &str = "e7bc546316d2d0ec13a2d3117b13468f5e939f95";
    static BAD_OID: &str = "deadbeefdeadbeefdead";

    fn kind() -> [u8; 2] {
        hex::decode("f5b7").unwrap().try_into().unwrap()
    }
    fn bad_kind() -> [u8; 2] {
        hex::decode("b5f9").unwrap().try_into().unwrap()
    }

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
            let cache = RocksDBCache::open(file_path.clone());
            cache
                .put(kind(), CacheKey::from_str(KEY).unwrap(), b"abcd")
                .unwrap();
        }
        (tmp_dir, file_path)
    }

    #[test]
    fn test_key_insert_get() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        std::thread::sleep(Duration::from_secs(2));
        let value = cache.get(kind(), CacheKey::from_str(KEY).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_key_insert_get_ttl() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open_with_ttl(file_path, Duration::from_secs(3600));
        let value = cache.get(kind(), CacheKey::from_str(KEY).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_key_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        let value = cache.get(kind(), CacheKey::from_str(BAD_OID).unwrap());
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_function_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        let value = cache.get(bad_kind(), CacheKey::from_str(KEY).unwrap());
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_compositekey() {
        let oid_bytes = CompositeKey {
            kind: kind(),
            key: CacheKey::from_str(KEY).unwrap(),
        }
        .to_bytes();
        let inflated_oid = CompositeKey::from_bytes(&oid_bytes).unwrap();
        assert_eq!(inflated_oid.key, CacheKey::from_str(KEY).unwrap());
        assert_eq!(inflated_oid.kind, kind());
    }

    #[test]
    fn test_oid_invariants() {
        assert_eq!(KEY_BYTE_LENGTH, CacheKey::zero().as_bytes().len());
    }

    #[test]
    fn test_compositekey_from_str() {
        assert_eq!(
            CompositeKey::from_str(
                format!("{}{}{}", hex::encode(kind()), DELIMITER, HEX_KEY).as_str()
            )
            .unwrap(),
            CompositeKey {
                kind: kind(),
                key: CacheKey::from_str(HEX_KEY).unwrap(),
            }
        );
    }
}
