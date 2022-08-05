// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};

use serde::{de::DeserializeOwned, Serialize};
use url::Url;

use super::*;

pub trait ProjectCacheBackend {
    // Fetch a model from the given URL and decode it from its JSON representation.
    fn load_model(&self, url: Url) -> Result<Vec<u8>>;

    fn store(&self, url: Url, value: Vec<u8>) -> Result<()>;

    fn endpoint(&self) -> Url;
}

pub trait ProjectCacheBackendInternal: Sync + Send {}

fn manifest_path(backend: &dyn ProjectCacheBackend, build_graph_hash: &Vec<u8>) -> Url {
    let mut url = backend.endpoint();
    let path = format!(
        "{}/{}.manifest_v{}.json",
        url.path(),
        hex::encode(&build_graph_hash).as_str(),
        PROJECT_CACHE_VERSION,
    );
    url.set_path(&path);
    url
}

fn export_path(
    backend: &dyn ProjectCacheBackend,
    build_graph_hash: &Vec<u8>,
    shard_index: usize,
    shard_count: usize,
) -> Url {
    let mut url = backend.endpoint();
    let path = format!(
        "{}/{}_{}_{}.export_v{}.json",
        url.path(),
        hex::encode(&build_graph_hash).as_str(),
        shard_index + 1,
        shard_count,
        PROJECT_CACHE_VERSION,
    );
    url.set_path(&path);
    url
}

/// Load and deserialize a model from the given backend.
fn load_model<T>(backend: &dyn ProjectCacheBackend, url: Url) -> Result<T>
where
    T: DeserializeOwned,
{
    backend
        .load_model(url)
        .and_then(|value| serde_json::from_slice(&value).map_err(anyhow::Error::new))
}

/// Serialize and store a model to the given backend.
fn store_model<T>(backend: &dyn ProjectCacheBackend, url: Url, value: &T) -> Result<()>
where
    T: Serialize,
{
    serde_json::to_vec(value)
        .map_err(anyhow::Error::new)
        .and_then(|v| backend.store(url, v))
}

/// Fetch all exports for the given build graph hash by reading the manifest and fetching each shard.
pub fn fetch_exports(
    backend: &dyn ProjectCacheBackend,
    build_graph_hash: &Vec<u8>,
) -> Result<Vec<Export>> {
    let span = tracing::info_span!("Fetching project cache data");
    let _guard = span.enter();
    // Fetch the manifest to determine how many shards there are.
    let manifest: ExportManifest = load_model(backend, manifest_path(backend, build_graph_hash))?;
    let mut exports = Vec::<Export>::with_capacity(manifest.shard_count);

    for shard_index in 0..manifest.shard_count {
        let export = load_model(
            backend,
            export_path(backend, build_graph_hash, shard_index, manifest.shard_count),
        )
        .with_context(|| {
            format!(
                "Failed to fetch shard {} of {}",
                shard_index + 1,
                manifest.shard_count
            )
        })?;
        exports.push(export);
    }

    Ok(exports)
}

/// Store an export to the given backend, writing a manifest explaining how many shards were produced if one has not been written. Fails if the shard count does not agree.
pub fn store_export(
    backend: &dyn ProjectCacheBackend,
    build_graph_hash: &Vec<u8>,
    manifest: &ExportManifest,
    export: &Export,
) -> Result<()> {
    let manifest_path = manifest_path(backend, build_graph_hash);
    let span = tracing::info_span!("Uploading project cache manifest");
    let _guard = span.enter();
    if let Ok(existing_manifest) = load_model::<ExportManifest>(backend, manifest_path.clone()) {
        // If a manifest exists, make sure that it is identical.
        if manifest.ne(&existing_manifest) {
            tracing::warn!(new_manifest = ?manifest, ?existing_manifest, "Manifests differ");
            bail!("Previously uploaded manifest does not match the local manifest");
        }
    } else {
        // Upload a manifest since it does not exist.
        store_model(backend, manifest_path, manifest).context("Failed to upload manifest")?;
    }

    store_model(
        backend,
        export_path(
            backend,
            build_graph_hash,
            export.shard_index,
            export.shard_count,
        ),
        export,
    )
    .context("Failed to store export")
}
