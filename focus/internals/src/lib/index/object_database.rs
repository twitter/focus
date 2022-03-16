use super::content_hash::HashContext;
use super::{ContentHash, DependencyKey, DependencyValue};

/// A persistent key-value cache mapping the hashes of [`super::DependencyKey`]s
/// to [`DependencyValue`]s.
pub trait ObjectDatabase {
    /// Look up the key-value pair with the provided key hash.
    fn get(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
    ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)>;

    /// Insert a new key-value pair into persistent storage.
    fn insert(
        &self,
        ctx: &HashContext,
        key: &DependencyKey,
        value: DependencyValue,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
pub mod testing {
    use tracing::{debug, error};

    use crate::index::{ContentHash, ContentHashable};

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
        fn get(
            &self,
            ctx: &HashContext,
            key: &DependencyKey,
        ) -> anyhow::Result<(ContentHash, Option<DependencyValue>)> {
            let hash = key.content_hash(ctx)?;
            let entries = self.entries.lock().expect("poisoned mutex");
            let dep_value = entries.get(&hash).cloned();
            Ok((hash, dep_value))
        }

        fn insert(
            &self,
            ctx: &HashContext,
            key: &DependencyKey,
            value: DependencyValue,
        ) -> anyhow::Result<()> {
            let hash = key.content_hash(ctx)?;
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
