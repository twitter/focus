use super::{ContentHash, DependencyValue};

/// A persistent key-value cache mapping the hashes of [`super::DependencyKey`]s
/// to [`DependencyValue`]s.
pub trait ObjectDatabase {
    /// Look up the key-value pair with the provided key hash.
    fn get(&self, hash: &ContentHash) -> anyhow::Result<Option<DependencyValue>>;

    /// Insert a new key-value pair into persistent storage.
    fn insert(&self, hash: ContentHash, value: DependencyValue) -> anyhow::Result<()>;
}

#[cfg(test)]
pub mod testing {
    use tracing::{debug, error};

    use super::*;

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Object database backed by an in-memory hashmap. This doesn't actually
    /// persist data between invocations of the program, so it's primarily
    /// useful for testing.
    #[derive(Clone, Debug)]
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
        fn get(&self, hash: &ContentHash) -> anyhow::Result<Option<DependencyValue>> {
            let entries = self.entries.lock().expect("poisoned mutex");
            Ok(entries.get(hash).cloned())
        }

        fn insert(&self, hash: ContentHash, value: DependencyValue) -> anyhow::Result<()> {
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
}
