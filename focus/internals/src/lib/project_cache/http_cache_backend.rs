// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use url::Url;

use super::*;

/// A cache backend that uses HTTP GET to retrieve models and HTTP PUT to store them.
pub struct HttpCacheBackend {
    endpoint: Url,
    client: Client,
}

impl HttpCacheBackend {
    pub fn new(endpoint: Url) -> Result<Self> {
        let client = Self::blocking_client().context("Creating HTTP client failed")?;
        Ok(Self { endpoint, client })
    }

    fn blocking_client() -> Result<Client> {
        // TODO: use vergen to get the SHA, cargo features, ...
        static APP_USER_AGENT: &str = concat!("focus", "/", env!("CARGO_PKG_VERSION"));
        Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(APP_USER_AGENT)
            .gzip(true)
            .build()
            .map_err(anyhow::Error::new)
    }
}

impl ProjectCacheBackend for HttpCacheBackend {
    fn endpoint(&self) -> Url {
        self.endpoint.clone()
    }

    // Fetch a model from the given URL and decode it from its JSON representation.
    fn load_model(&self, url: Url) -> Result<Vec<u8>> {
        let span = tracing::info_span!("Fetching");
        let _guard = span.enter();
        tracing::debug!(url = ?url.as_str(), "GET");
        let mut buf = Vec::<u8>::new();
        {
            let mut response = self
                .client
                .get(url)
                .send()
                .context("GET failed")?
                .error_for_status()?;
            tracing::debug!(status = ?response.status(), "OK");
            response
                .read_to_end(&mut buf)
                .context("Reading response failed")?;
        }
        Ok(buf)
    }

    // Encode the given model as JSON and upload it using HTTP PUT to the given URL.
    fn store(&self, url: Url, value: Vec<u8>) -> Result<()> {
        // TODO: Add an ETag to skip upload if the content is identical.
        let span = tracing::info_span!("Putting");
        let _guard = span.enter();
        tracing::debug!(url = ?url.as_str(), "PUT");
        let response = self
            .client
            .put(url)
            .body(value)
            .send()
            .context("PUT failed")?
            .error_for_status()?;
        tracing::debug!(status = ?response.status(), "OK");
        Ok(())
    }
}
