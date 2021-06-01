use anyhow::Result;
use log::info;
use protobuf::CodedOutputStream;
use rocksdb::{Direction, IteratorMode, Options, DB};
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
        let mut db = DB::open(&opts, &path)
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
        let mut iter = self.general.prefix_iterator(key);
        for (sub_key, value) in iter {
            let suffix = &sub_key[key.len()..];
            results.insert(Vec::from(suffix), Vec::from(value));
        }
        Ok(results)
    }

    pub fn get_bytes(&self, key: &[u8]) -> Result<Option<Vec<u8>>, AppError> {
        {
            let snapshot = self.general.snapshot();
            match snapshot.get(&key) {
                Ok(Some(bytes)) => {
                    return Ok(Some(bytes));
                }
                Ok(None) => {
                    return Ok(None);
                }
                Err(err) => return Err(AppError::Db(err)),
            }
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
                    let val = T::parse_from_bytes(bytes.as_slice())?;
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

#[cfg(test)]
mod tests {
    use crate::error::AppError;

    #[test]
    fn smoke() -> Result<(), AppError> {
        Ok(())
    }
}
