use std::default::Default;
use std::string::ToString;
use std::time::Duration;
use std::{cell::RefCell, fmt::Debug, str::FromStr};

use anyhow::{self, Context};
use rocksdb::{Options, DB};
use std::path::{Path, PathBuf};

pub type Key = git2::Oid;

pub trait Cache: Debug {
    fn put(&self, function_id: Key, argument: Key, value: &[u8]) -> anyhow::Result<()>;
    fn get(&self, function_id: Key, argument: Key) -> anyhow::Result<Option<Vec<u8>>>;
    fn clear(&self) -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct RocksDBCache {
    db: RefCell<Option<DB>>,
    ttl: Duration,
}

#[derive(Debug)]
pub struct CompositeKey {
    pub argument: Key,
    pub function_id: Key,
}

impl PartialEq for CompositeKey {
    fn eq(&self, other: &Self) -> bool {
        self.argument == other.argument && self.function_id == other.function_id
    }
}

#[derive(Debug, Clone)]
pub enum ParseCompositeKeyError {
    NoMatch,
    MissingPrefix,
    MissingFunctionIdentifier,
    MissingArgument,
    MissingDelimeter,
}

impl ToString for CompositeKey {
    fn to_string(&self) -> String {
        format!("oid{},{}", self.argument, self.function_id)
    }
}

const KEY_PREFIX_LENGTH: usize = 3;
const KEY_PREFIX: &[u8; KEY_PREFIX_LENGTH] = b"oid";
const KEY_PREFIX_STR: &str = "oid";
const OID_BYTE_LENGTH: usize = 20;
const COMPOSITE_KEY_LENGTH: usize = KEY_PREFIX_LENGTH + OID_BYTE_LENGTH + OID_BYTE_LENGTH;
const HEX_ENCODED_COMPOSITE_KEY_LENGTH: usize =
    KEY_PREFIX_LENGTH + (OID_BYTE_LENGTH * 2) + 1 /* DELIMITER ',' */ + (OID_BYTE_LENGTH * 2);
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
            return Err(ParseCompositeKeyError::MissingPrefix);
        }
        let function_id =
            match Key::from_bytes(&s[KEY_PREFIX_LENGTH..KEY_PREFIX_LENGTH + OID_BYTE_LENGTH]) {
                Ok(oid) => oid,
                Err(_) => return Err(ParseCompositeKeyError::MissingFunctionIdentifier),
            };
        let argument =
            match Key::from_bytes(&s[KEY_PREFIX_LENGTH + OID_BYTE_LENGTH..COMPOSITE_KEY_LENGTH]) {
                Ok(oid) => oid,
                Err(_) => return Err(ParseCompositeKeyError::MissingArgument),
            };
        Ok(CompositeKey {
            function_id,
            argument,
        })
    }
}

impl FromStr for CompositeKey {
    type Err = ParseCompositeKeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ix: usize = 0;
        if s.len() != HEX_ENCODED_COMPOSITE_KEY_LENGTH {
            return Err(ParseCompositeKeyError::NoMatch);
        };
        if !s.starts_with(KEY_PREFIX_STR) {
            return Err(ParseCompositeKeyError::MissingPrefix);
        }
        ix += KEY_PREFIX_STR.len();
        let argument = match Key::from_str(&s[ix..ix + (OID_BYTE_LENGTH * 2)]) {
            Ok(oid) => oid,
            Err(_) => return Err(ParseCompositeKeyError::MissingArgument),
        };
        ix += OID_BYTE_LENGTH * 2;
        if &s[ix..ix + 1] != "," {
            return Err(ParseCompositeKeyError::MissingDelimeter);
        }
        ix += 1;
        let function_id = match Key::from_str(&s[ix..ix + (OID_BYTE_LENGTH * 2)]) {
            Ok(oid) => oid,
            Err(_) => return Err(ParseCompositeKeyError::MissingFunctionIdentifier),
        };

        Ok(CompositeKey {
            argument,
            function_id,
        })
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
    fn put(&self, function_id: Key, argument: Key, value: &[u8]) -> anyhow::Result<()> {
        let key: &[u8] = &CompositeKey {
            function_id,
            argument,
        }
        .to_bytes()[..];
        self.db
            .borrow()
            .as_ref()
            .unwrap()
            .put(key, value)
            .with_context(|| format!("Putting {:?} failed", key))
    }

    fn get(&self, function_id: Key, argument: Key) -> anyhow::Result<Option<Vec<u8>>> {
        let key: &[u8] = &CompositeKey {
            function_id,
            argument,
        }
        .to_bytes()[..];
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

    use crate::{local_cache::OID_BYTE_LENGTH, Cache, CompositeKey, Key, RocksDBCache};

    static ARG: &str = "12345678912345789ab";
    static HEX_ARG: &str = "e7bc546316d2d0ec13a2d3117b13468f5e939f95";
    static FN_ID: &str = "abcd5abcd5abcd5abcd5";
    static HEX_FN_ID: &str = "f572d396fae9206628714fb2ce00f72e94f2258f";
    static BAD_OID: &str = "deadbeefdeadbeefdead";

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
                .put(
                    Key::from_str(FN_ID).unwrap(),
                    Key::from_str(ARG).unwrap(),
                    b"abcd",
                )
                .unwrap();
        }
        (tmp_dir, file_path)
    }

    #[test]
    fn test_key_insert_get() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        std::thread::sleep(Duration::from_secs(2));
        let value = cache.get(Key::from_str(FN_ID).unwrap(), Key::from_str(ARG).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_key_insert_get_ttl() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open_with_ttl(file_path, Duration::from_secs(3600));
        let value = cache.get(Key::from_str(FN_ID).unwrap(), Key::from_str(ARG).unwrap());
        assert_eq!(value?.unwrap(), b"abcd".to_vec());
        Ok(())
    }

    #[test]
    fn test_arg_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        let value = cache.get(
            Key::from_str(FN_ID).unwrap(),
            Key::from_str(BAD_OID).unwrap(),
        );
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_function_missing() -> anyhow::Result<()> {
        let (_temp_dir, file_path) = create_test_repo();
        let cache = RocksDBCache::open(file_path);
        let value = cache.get(Key::from_str(BAD_OID).unwrap(), Key::from_str(ARG).unwrap());
        assert_eq!(value?, None);
        Ok(())
    }

    #[test]
    fn test_compositekey() {
        let oid_bytes = CompositeKey {
            argument: Key::from_str(ARG).unwrap(),
            function_id: Key::from_str(FN_ID).unwrap(),
        }
        .to_bytes();
        let inflated_oid = CompositeKey::from_bytes(&oid_bytes).unwrap();
        assert_eq!(inflated_oid.argument, Key::from_str(ARG).unwrap());
        assert_eq!(inflated_oid.function_id, Key::from_str(FN_ID).unwrap());
    }

    #[test]
    fn test_oid_invariants() {
        assert_eq!(OID_BYTE_LENGTH, Key::zero().as_bytes().len());
    }

    #[test]
    fn test_compositekey_from_str() {
        assert_eq!(
            CompositeKey::from_str(format!("oid{},{}", HEX_ARG, HEX_FN_ID).as_str()).unwrap(),
            CompositeKey {
                argument: Key::from_str(HEX_ARG).unwrap(),
                function_id: Key::from_str(HEX_FN_ID).unwrap(),
            }
        );
    }
}
