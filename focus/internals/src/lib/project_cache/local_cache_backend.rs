// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Borrow;

use anyhow::{Context, Result};
use url::Url;

use super::*;

#[derive(thiserror::Error, Debug)]
pub enum DatabaseCacheBackendError {
    #[error("Local cache backend requires a 'file' scheme in the endpoint URL")]
    InappropriateScheme,
}

/// A cache backend that stores and retrieves content to and from a database.
pub struct LocalCacheBackend {
    pub endpoint: Url,
    pub database: rocksdb::DB,
}

impl LocalCacheBackend {
    pub fn new(endpoint: Url) -> Result<Self> {
        if !endpoint.scheme().eq_ignore_ascii_case("file") {
            return Err(anyhow::Error::new(
                DatabaseCacheBackendError::InappropriateScheme,
            ));
        }

        let path = endpoint
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("Endpoint is missing path"))
            .context("Converting the endpoint to a local path failed")?;
        let database = storage::open_database(&path, Duration::from_secs(84600))
            .context("Opening local project cache database")?;

        Ok(Self { endpoint, database })
    }
}

impl ProjectCacheBackend for LocalCacheBackend {
    fn load_model(&self, url: Url) -> Result<Vec<u8>> {
        let key = url.path();
        if let Ok(Some(repr)) = self.database.borrow().get(&key) {
            debug!(?key, "GET: Found");
            Ok(repr)
        } else {
            warn!(?key, "GET: Missing");
            Err(anyhow::anyhow!("Missing"))
        }
    }

    fn store(&self, url: Url, value: Vec<u8>) -> Result<()> {
        let key = url.path();
        debug!(?key, "PUT");
        self.database
            .put(&key, value)
            .with_context(|| format!("Writing key '{}' failed", &key))
    }

    fn endpoint(&self) -> Url {
        self.endpoint.clone()
    }
}
