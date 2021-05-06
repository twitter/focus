use crate::error::AppError;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::RwLock;

#[derive(Debug)]
pub struct TokenTable<K>
where
    K: Hash + Eq + PartialEq + Clone,
{
    forward: RwLock<HashMap<K, usize>>,
    reverse: RwLock<Vec<K>>,
}

impl<K> TokenTable<K>
where
    K: Hash + Eq + PartialEq + Clone,
{
    pub fn new() -> Self {
        Self {
            forward: RwLock::new(HashMap::<K, usize>::new()),
            reverse: RwLock::new(Vec::<K>::new()),
            // TODO: Remove data duplication here by figuring out the right reference
        }
    }

    // Look up an entry, optionally inserting it. Returns a tuple of the associated index and boolean indicating if
    // it was created.
    // TODO: Make this accept values convertable to K
    pub fn forward(&self, key: &K, allow_create: bool) -> Result<(usize, bool), AppError> {
        // Read and return
        {
            if let Ok(reader) = self.forward.read() {
                if let Some(val) = reader.get(key) {
                    return Ok((*val, false));
                }
            } else {
                return Err(AppError::ReadLockFailed());
            }
        }

        if !allow_create {
            return Err(AppError::Missing());
        }

        // Create the item
        if let Ok(mut reverse_writer) = self.reverse.write() {
            if let Ok(mut forward_writer) = self.forward.write() {
                let assigned_index = reverse_writer.len();
                let owned_key = key.clone();
                reverse_writer.push(owned_key.clone());
                forward_writer.insert(owned_key, assigned_index);
                return Ok((assigned_index, true));
            }
        }

        // Write lock not obtained
        Err(AppError::WriteLockFailed())
    }

    pub fn reverse(&self, index: usize) -> Result<K, AppError> {
        if let Ok(reader) = self.reverse.read() {
            if let Some(val) = reader.get(index) {
                return Ok(val.clone());
            } else {
                return Err(AppError::Missing());
            }
        } else {
            return Err(AppError::ReadLockFailed());
        }
    }
}

#[test]
fn smoke() -> Result<(), AppError> {
    let tab = TokenTable::<String>::new();

    let key1 = String::from("hi");
    let key2 = String::from("hiya");
    assert!(tab.forward(&key1, false).is_err()); // Create denied
    assert_eq!(tab.forward(&key1, true)?, (0, true)); // Create value
    assert_eq!(tab.forward(&key1, false)?, (0, false)); // Retrieve value
    assert_eq!(tab.reverse(0)?, "hi"); // Reverse lookup
    assert_eq!(tab.forward(&key1, true)?, (0, false)); // Retrieve value
    assert_eq!(tab.forward(&key2, true)?, (1, true)); // Distinct value regardless of prefix (we may compress data later with a trie or similar)
    assert_eq!(tab.reverse(1)?, "hiya"); // Reverse lookup
    assert!(tab.reverse(2).is_err());

    Ok(())
}
