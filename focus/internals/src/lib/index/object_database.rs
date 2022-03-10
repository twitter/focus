use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{debug, error};

use super::{ContentHash, DependencyValue};

/// A persistent key-value cache mapping the hashes of [`super::DependencyKey`]s
/// to [`DependencyValue`]s.
#[derive(Clone, Debug)]
pub struct ObjectDatabase {
    entries: Arc<Mutex<HashMap<ContentHash, DependencyValue>>>,
}

impl ObjectDatabase {
    /// Constructor.
    #[allow(clippy::new_without_default)] // expecting to change this constructor later
    pub fn new() -> Self {
        ObjectDatabase {
            entries: Default::default(),
        }
    }

    /// Look up the key-value pair with the provided key hash.
    pub fn get(&self, hash: impl Borrow<ContentHash>) -> anyhow::Result<Option<DependencyValue>> {
        let entries = self.entries.lock().expect("poisoned mutex");
        Ok(entries.get(hash.borrow()).cloned())
    }

    /// Insert a new key-value pair into persistent storage.
    pub fn insert(&self, hash: ContentHash, value: DependencyValue) -> anyhow::Result<()> {
        debug!(?hash, ?value, "Inserting entry into object database");

        let mut entries = self.entries.lock().expect("poisoned mutex");
        if let Some(old_value) = entries.insert(hash.clone(), value.clone()) {
            if value != old_value {
                error!(
                    ?old_value,
                    new_value = ?value,
                    ?hash,
                    "Non-deterministic dependency hashing"
                );
            }
        }
        Ok(())
    }
}
