use anyhow::Result;
use log::info;
use protobuf::CodedOutputStream;
use rocksdb::{Options, DB};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::AppError;

#[derive(Debug)]
pub struct Storage {
    path: PathBuf,
    general: Arc<DB>,
}

impl Storage {
    pub fn new(path: &Path) -> Result<Storage, AppError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.increase_parallelism(4);
        opts.set_max_open_files(100);
        info!("Opening database from '{}'", &path.display());
        let db = DB::open(&opts, &path)
            .expect(format!("Failed to open DB from '{}'", &path.display()).as_str());
        info!("Database is ready");
        Ok(Storage {
            path: path.to_path_buf(),
            general: Arc::new(db),
        })
    }

    pub fn put_bytes(&self, key: &[u8], value: &[u8]) -> Result<(), AppError> {
        self.general.put(key, value).map_err(|e| AppError::Db(e))
    }

    pub fn put<T>(&self, key: &[u8], value: &T) -> Result<(), AppError>
    where
        T: ::protobuf::Message,
    {
        let mut buf = Vec::<u8>::new();
        let mut out = CodedOutputStream::bytes(buf.as_mut_slice());
        value.write_to(&mut out)?;
        self.put_bytes(key, &buf)
    }

    /// Read bytes associated with `key`.
    pub fn get_by_prefix(&self, key: &[u8]) -> Result<HashMap<Vec<u8>, Vec<u8>>, AppError> {
        let mut results = HashMap::<Vec<u8>, Vec<u8>>::new();
        let iter = self.general.prefix_iterator(key);
        for (sub_key, value) in iter {
            let suffix = &sub_key[key.len()..];
            results.insert(Vec::from(suffix), Vec::from(value));
        }
        Ok(results)
    }

    pub fn get_bytes(&self, key: &[u8]) -> Result<Option<Box<[u8]>>, AppError> {
        let snapshot = self.general.snapshot();
        match snapshot.get(&key) {
            Ok(Some(bytes)) => Ok(Some(bytes.into_boxed_slice())),
            Ok(None) => Ok(None),
            Err(err) => Err(AppError::Db(err)),
        }
    }

    /// Read the value associated with `key` or insert and return the result by running `fault_fn`.
    pub fn get_or_fault<T, F>(&self, key: Vec<u8>, fault_fn: F) -> Result<Option<T>, AppError>
    where
        T: ::protobuf::Message,
        F: FnOnce() -> Result<Option<T>, AppError>,
    {
        {
            match self.get_bytes(&key) {
                Ok(Some(bytes)) => {
                    let val = T::parse_from_bytes(&bytes)?;
                    return Ok(Some(val));
                }
                Ok(None) => {
                    // Fault below.
                }
                Err(err) => return Err(err),
            }
        }

        match fault_fn() {
            Ok(Some(value)) => {
                self.put(&key, &value)?;
                Ok(Some(value))
            }
            // TODO: Consider negative caching. It seems undesirable.
            otherwise => otherwise,
        }
    }

    pub fn estimate_num_keys(&self) -> Result<i64, AppError> {
        if let Ok(Some(val)) = self.general.property_value("rocksdb.estimate-num-keys") {
            Ok(val.parse::<i64>()?)
        } else {
            return Err(AppError::None());
        }
    }
}

const DEFAULT_NS: u8 = b'o';
const DATA_VERSION: u8 = 1;
const DEFAULT_SEP: u8 = b':';

#[allow(dead_code)]
const COMMIT_KEY_BYTE: u8 = b'c';
#[allow(dead_code)]
const TREE_KEY_BYTE: u8 = b't';
#[allow(dead_code)]
const BLOB_KEY_BYTE: u8 = b'b';
#[allow(dead_code)]
const TAG_KEY_BYTE: u8 = b'g';

const HEADER_KEY_BYTE: u8 = b'h';
const BODY_KEY_BYTE: u8 = b'b';

#[derive(Debug)]
pub struct Keygen {
    // namespace byte "o" by defautt
    ns: u8,
    // separator character
    sep: u8,
    // data version
    version: u8,
}

impl Keygen {
    pub fn key_for(&self, oid: &[u8]) -> Key {
        Key {
            ns: self.ns,
            sep: self.sep,
            version: self.version,
            oid: Vec::from(oid),
        }
    }

    pub fn default() -> Keygen {
        Keygen {
            ns: DEFAULT_NS,
            sep: DEFAULT_SEP,
            version: DATA_VERSION,
        }
    }
}

#[derive(Debug)]
pub struct Key {
    ns: u8,
    sep: u8,
    version: u8,
    oid: Vec<u8>,
}

impl Key {
    /// A key without the 'header' or 'body' component. Can be used to retrieve
    /// all keys for a given oid
    pub fn base(&self) -> Vec<u8> {
        let mut v = Vec::<u8>::new();
        v.push(self.sep);
        v.push(self.version);
        v.push(self.sep);
        v.extend(&self.oid);
        v
    }

    pub fn for_header(&self) -> Box<[u8]> {
        let mut v = self.base();
        v.push(self.sep);
        v.push(HEADER_KEY_BYTE);
        v.into_boxed_slice()
    }

    pub fn for_body(&self) -> Box<[u8]> {
        let mut v = self.base();
        v.push(self.sep);
        v.push(BODY_KEY_BYTE);
        v.into_boxed_slice()
    }
}

#[cfg(test)]
mod tests {
    use crate::error::AppError;

    #[test]
    fn smoke() -> Result<(), AppError> {
        Ok(())
    }
}
