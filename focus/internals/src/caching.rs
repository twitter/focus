use crate::error::AppError;
use anyhow::{Error, Result};
use crossbeam_utils::sync::ShardedLock;
use log::{debug, warn};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::thread::{spawn, JoinHandle};
use std::time::{Duration, Instant};
use std::{borrow::Borrow, cell::RefCell, sync::Arc};

pub struct TtlCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    entries: ShardedLock<HashMap<K, (V, Instant)>>,
    ttl: Option<Duration>,
    last_scavenge: Mutex<Instant>,
}

impl<K, V> TtlCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new(ttl: Option<Duration>) -> Self {
        Self {
            entries: ShardedLock::new(HashMap::<K, (V, Instant)>::new()),
            ttl,
            last_scavenge: Mutex::new(Instant::now()),
        }
    }

    pub fn get(&mut self, key: &K, now: Instant) -> Result<Option<V>, AppError> {
        let mut evict = false;
        if let Ok(reader) = self.entries.read() {
            if let Some((val, expiry)) = reader.get(key) {
                if self.ttl.is_none() || expiry > &now {
                    return Ok(Some(val.clone()));
                } else {
                    evict = true;
                }
            }
        } else {
            return Err(AppError::ReadLockFailed());
        }

        if evict {
            if let Ok(mut writer) = self.entries.write() {
                writer.remove(&key);
            }
        }

        Ok(None)
    }

    pub fn insert(&mut self, k: K, v: V, now: Instant) -> Result<Option<(V, Instant)>, AppError> {
        let expiry = now
            .checked_add(self.ttl.unwrap_or(Duration::from_nanos(0)))
            .expect("Calculating TTL failed");
        self.scavenge(now);
        if let Ok(mut writer) = self.entries.write() {
            Ok(writer.insert(k, (v, expiry)))
        } else {
            Err(AppError::ReadLockFailed())
        }
    }

    pub fn get_or_fault<F>(
        &mut self,
        key: &K,
        fault: F,
        now: Instant,
    ) -> Result<Option<V>, AppError>
    where
        F: Fn(&K) -> Result<V, AppError>,
    {
        match self.get(key, now) {
            Ok(Some(result)) => Ok(Some(result)),
            _ => {
                let faulted_value = fault(key);
                if let Ok(value) = &faulted_value {
                    self.insert(key.clone(), value.clone(), now)?;
                }
                Ok(Some(faulted_value.unwrap()))
            }
        }
    }

    fn scavenge(&mut self, now: Instant) {
        if self.ttl.is_none() {
            return;
        }

        let mtx = self.last_scavenge.try_lock();
        if mtx.is_err() {
            debug!("Did not acquire lock.");
            return;
        }
        let mut scavenge_lock = mtx.unwrap();
        if now
            .checked_duration_since(*scavenge_lock)
            .map(|dur| dur < self.ttl.unwrap())
            .unwrap_or(false)
        {
            // It was too recent. Skip it.
            return;
        }

        // let thread_id = std::thread::current().id();
        debug!(
            "Scavenge attempt initiated on {:?}",
            std::thread::current().id()
        );

        let mut removed_count = 0;
        let mut marked = Vec::<K>::new();

        // Mark expired keys
        {
            if let Ok(reader) = self.entries.read() {
                for (k, (_, _)) in reader.iter().filter(|(_, (_, expiry))| expiry < &now) {
                    marked.push(k.clone());
                }
            } else {
                warn!("[WEIRD] Could not obtain read lock.");
            }
        }

        if marked.is_empty() {
            debug!("[WEIRD] Scavenge aborted; no marked items.");
            // This is kind of a bad sign.
            return;
        }

        // Sweep suspected expired keys, checking they are still invalid before removing them
        if let Ok(mut writer) = self.entries.write() {
            for key in &marked {
                if let Some((_, ttl)) = writer.get(&key) {
                    if ttl < &now {
                        writer.remove(&key);
                        removed_count += 1;
                    }
                }
            }
        } else {
            warn!("[WEIRD] Could not obtain write lock.");
        }

        debug!(
            "Swept {} expired entries of the {} initially marked",
            removed_count,
            &marked.len()
        );

        *scavenge_lock = now;
    }
}
