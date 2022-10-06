// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

mod http_cache_backend;
mod local_cache_backend;

mod remote;
pub use http_cache_backend::HttpCacheBackend;
pub use local_cache_backend::LocalCacheBackend;
pub use remote::ProjectCacheBackend;
mod model;
pub(crate) use model::{Export, ExportManifest, Key, RepoIdentifier, Value};

use tracing::{debug, info, info_span, warn};

use std::{
    collections::{BTreeSet, HashSet},
    convert::{TryFrom, TryInto},
    ffi::OsStr,
    iter::FromIterator,
    os::unix::prelude::OsStrExt,
    path::Path,
    sync::Arc,
    time::Duration,
};

use crate::model::{
    data_paths::DataPaths,
    outlining::PatternSet,
    repo::Repo,
    selection::{Target, TargetSet},
};
use anyhow::{bail, Context};
use focus_util::{app::App, paths::is_relevant_to_build_graph};
use git2::{ObjectType, Oid, TreeWalkMode, TreeWalkResult};
use lazy_static::lazy_static;
use rocksdb::WriteBatch;
use sha2::{Digest, Sha256};
use url::Url;

use crate::storage;

use self::{
    model::NamespacedKey,
    remote::{fetch_exports, store_export},
};

const PROJECT_CACHE_VERSION: usize = 1;

lazy_static! {
    static ref VERSION_KEY_SUFFIX: String = format!(";V{}", PROJECT_CACHE_VERSION);
    static ref VERSION_KEY_PATH_COMPONENT: String = format!("v{}", PROJECT_CACHE_VERSION);
    static ref IMPORT_RECEIPT_IOTA_SERIALIZED: Vec<u8> =
        serde_json::to_vec(&Value::ImportReceiptIota).unwrap();
}
const PROJECT_CACHE_TTL: Duration = Duration::new(86400 * 14, 0);

/// ProjectCache caches pattern sets for projects. This is a coarse-grained cache intended to
/// be used when there are no ad-hoc targets present in the selection. It is a bit of a kludge
/// and meant to be support fast synchronization until we can work through all of the correctness
/// and performance issues with the more accurate and fine-grained precomputed index.
pub struct ProjectCache<'cache> {
    app: Arc<App>,
    repo: &'cache Repo,
    identifier: RepoIdentifier,
    database: rocksdb::DB,
    backend: Box<dyn ProjectCacheBackend>,
}

impl<'cache> ProjectCache<'cache> {
    /// Create a new project cache instance for the provided Repo.
    pub fn new(repo: &'cache Repo, endpoint: Url, app: Arc<App>) -> anyhow::Result<Self> {
        let identifier = RepoIdentifier::from(repo.underlying())?;
        let working_tree = repo
            .working_tree()
            .ok_or_else(|| anyhow::anyhow!("Missing working tree"))?;
        let data_paths = DataPaths::from_working_tree(working_tree)?;
        let database = {
            let span = info_span!("Opening project cache");
            let _guard = span.enter();
            let database_path = data_paths.project_cache_dir.as_path();
            let result = storage::open_database(database_path, PROJECT_CACHE_TTL)
                .context("Opening project cache database")?;
            debug!(?database_path, "Database is open");
            result
        };
        let backend = Self::make_backend(&endpoint)?;
        Ok(Self {
            app,
            repo,
            identifier,
            database,
            backend,
        })
    }

    /// Read a value from the cache, possibly faulting it using the optional callback.
    #[allow(clippy::type_complexity)] // Can't do anything about `fault_cb` because of the ref.
    pub(crate) fn read_or_fault(
        &self,
        key: &Key,
        fault_cb: Option<&dyn Fn(&Key, &Repo) -> anyhow::Result<Option<Value>>>,
    ) -> anyhow::Result<(NamespacedKey, Option<Value>)> {
        let outer_key = NamespacedKey {
            repository: self.identifier.clone(),
            underlying: key.to_owned(),
            version: PROJECT_CACHE_VERSION,
        };
        let key_str: String = outer_key.clone().try_into()?;

        if let Some(value_slice) = self
            .database
            .get_pinned(key_str.as_bytes())
            .with_context(|| format!("Reading value '{}' failed", &key_str))?
        {
            // A value was found, deserialize it and return it.
            let value: Value =
                serde_json::from_slice(&value_slice).context("Parsing value failed")?;
            return Ok((outer_key, Some(value)));
        }

        if fault_cb.is_none() {
            warn!(key = ?key_str, "Not found; cannot be faulted");
            bail!("Object not found and cannot be faulted")
        }

        // Fault the value from the given callback and store it
        let span = info_span!("Faulting");
        info!(key = ?key_str, "Not found; faulting");
        let _guard = span.enter();
        if let Some(value) = fault_cb.unwrap()(key, self.repo)
            .with_context(|| format!("Faulting object for key '{}'", key))?
        {
            debug!(?value, "Faulted object");
            let serialized_value = serde_json::to_vec(&value).with_context(|| {
                format!(
                    "Serializing  value {:?} for key '{}' failed",
                    &value, &key_str
                )
            })?;
            self.database
                .put(key_str.as_bytes(), serialized_value)
                .with_context(|| format!("Writing value '{}' failed", key_str))?;
            Ok((outer_key, Some(value)))
        } else {
            // Fault function returned none
            warn!("Fault returned nothing");
            Ok((outer_key, None))
        }
    }

    /// Get or calculate the build graph hash at a given commit.
    pub fn build_graph_hash(
        &self,
        commit_id: Oid,
        allow_fault: bool,
    ) -> anyhow::Result<(NamespacedKey, Option<Value>)> {
        let key = Key::CommitToBuildGraphHash {
            commit_id: commit_id.as_bytes().to_vec(),
        };

        let calculate = move |key: &Key, repo: &Repo| -> anyhow::Result<Option<Value>> {
            match key {
                Key::CommitToBuildGraphHash { commit_id } => {
                    let git_repo = repo.underlying();
                    let commit_id =
                        Oid::from_bytes(commit_id).context("Marshalling commit ID failed")?;
                    let commit = git_repo
                        .find_commit(commit_id)
                        .context("Resolving commit failed")?;
                    let tree = commit.tree().context("Resolving tree failed")?;

                    let mut digest = Sha256::new();
                    tree.walk(TreeWalkMode::PreOrder, |directory, entry| {
                        if entry
                            .kind()
                            .map(|kind| kind == ObjectType::Blob)
                            .unwrap_or(false)
                        {
                            let filename = Path::new(OsStr::from_bytes(entry.name_bytes()));
                            if is_relevant_to_build_graph(filename) {
                                digest.update(directory.as_bytes());
                                digest.update(entry.name_bytes());
                                digest.update(entry.id().as_bytes());
                            }
                        }

                        TreeWalkResult::Ok
                    })
                    .context("Walking tree failed")?;

                    Ok(Some(Value::BuildGraphHash {
                        build_graph_hash: digest.finalize().to_vec(),
                    }))
                }
                _ => Err(anyhow::anyhow!("Unexpected key type {:?}", &key)),
            }
        };

        self.read_or_fault(&key, if allow_fault { Some(&calculate) } else { None })
    }

    /// Outling the given targets at a specific commit.
    fn outline(&self, commit_id: Oid, targets: &HashSet<Target>) -> anyhow::Result<PatternSet> {
        let outlining_tree = self
            .repo
            .outlining_tree()
            .ok_or_else(|| anyhow::anyhow!("Missing outlining tree"))?;
        let (patterns, _resolution_result) = outlining_tree
            .outline(commit_id, targets, self.app.clone())
            .with_context(|| {
                format!(
                    "Outlining targets ({:?}) at commit {} failed",
                    &targets, commit_id
                )
            })?;
        Ok(patterns)
    }

    pub fn get_build_graph_hash(
        &self,
        commit_id: Oid,
        allow_fault: bool,
    ) -> anyhow::Result<(NamespacedKey, Vec<u8>)> {
        let (build_graph_hash_key, build_graph_hash) = self
            .build_graph_hash(commit_id, allow_fault)
            .context("Determining build graph hash")?;
        if let Some(Value::BuildGraphHash {
            build_graph_hash: value,
        }) = build_graph_hash
        {
            Ok((build_graph_hash_key, value))
        } else {
            Err(anyhow::anyhow!("Unexpected build graph hash value type"))
        }
    }

    /// Generate all projects in the given shard.
    pub fn generate_all(
        &self,
        commit_id: Oid,
        shard_index: usize,
        shard_count: usize,
    ) -> anyhow::Result<Vec<NamespacedKey>> {
        let selection_manager = self.repo.selection_manager()?;
        let catalog = selection_manager.project_catalog();
        let project_names = catalog
            .optional_projects
            .underlying
            .iter()
            .map(|(name, _project)| name.clone())
            .collect::<BTreeSet<String>>();
        let project_names = Vec::from_iter(project_names.into_iter());
        let (build_graph_hash_key, build_graph_hash) =
            self.get_build_graph_hash(commit_id, true)?;
        let mut keys = vec![build_graph_hash_key];
        let shard_size = project_names.len() / shard_count;
        let mut shards = project_names.chunks(shard_size).skip(shard_index);
        let build_graph_hash_str = hex::encode(&build_graph_hash);
        let commit_id_str = hex::encode(commit_id.as_bytes());
        let project_names = shards
            .next()
            .ok_or_else(|| anyhow::anyhow!("No such shard"))?;
        info!(
            ?shard_index,
            ?shard_count,
            ?project_names,
            commit_id = ?commit_id_str,
            build_graph_hash = ?build_graph_hash_str,
            "Generating project pattern cache"
        );
        for project_name in project_names {
            let (key, value) = self
                .get_optional_project_patterns(commit_id, &build_graph_hash, project_name, true)
                .with_context(|| {
                    format!("Generating cache content for project {}", project_name)
                })?;
            let _value = value.ok_or_else(|| {
                anyhow::anyhow!(
                    "Could not generate patterns to cache for project '{}'",
                    project_name
                )
            })?;
            keys.push(key);
        }

        Ok(keys)
    }

    pub fn get_mandatory_project_patterns(
        &self,
        commit_id: Oid,
        build_graph_hash: &[u8],
        fault: bool,
    ) -> anyhow::Result<(NamespacedKey, Option<Value>)> {
        let calculate = move |_key: &Key, repo: &Repo| -> anyhow::Result<Option<Value>> {
            let selection_manager = repo.selection_manager()?;
            let catalog = selection_manager.project_catalog();
            let span = info_span!("Resolving mandatory projects");
            let _guard = span.enter();

            let mut targets = TargetSet::new();
            for project in catalog.mandatory_projects.underlying.values() {
                targets.extend(TargetSet::try_from(project)?);
            }

            if let Ok(patterns) = self.outline(commit_id, &targets) {
                Ok(Some(Value::MandatoryProjectPatternSet(patterns)))
            } else {
                Ok(None)
            }
        };

        let key = Key::MandatoryProjectPatternSet {
            build_graph_hash: build_graph_hash.to_vec(),
        };
        self.read_or_fault(&key, if fault { Some(&calculate) } else { None })
    }

    /// Get (possibly fault) a project from the cache.
    pub fn get_optional_project_patterns(
        &self,
        commit_id: Oid,
        build_graph_hash: &[u8],
        project_name: &str,
        fault: bool,
    ) -> anyhow::Result<(NamespacedKey, Option<Value>)> {
        let calculate = move |key: &Key, repo: &Repo| -> anyhow::Result<Option<Value>> {
            if let Key::OptionalProjectPatternSet { project_name, .. } = key {
                let selection_manager = repo.selection_manager()?;
                let catalog = selection_manager.project_catalog();
                let span = info_span!("Resolving optional project");
                let _guard = span.enter();

                let project = catalog
                    .optional_projects
                    .underlying
                    .get(project_name)
                    .ok_or_else(|| {
                        anyhow::anyhow!(format!("No such project '{}'", &project_name))
                    })?;
                let targets = TargetSet::try_from(project)?;

                info!(project = ?project_name, "Outlining");
                if let Ok(patterns) = self.outline(commit_id, &targets) {
                    Ok(Some(Value::OptionalProjectPatternSet(patterns)))
                } else {
                    Ok(None)
                }
            } else {
                Err(anyhow::anyhow!("Unsupported key type"))
            }
        };

        let key = Key::OptionalProjectPatternSet {
            build_graph_hash: build_graph_hash.to_vec(),
            project_name: project_name.to_owned(),
        };
        self.read_or_fault(&key, if fault { Some(&calculate) } else { None })
    }

    /// Generate and push sharded project cache data.
    pub fn generate_and_push(
        &self,
        keys: &Vec<NamespacedKey>,
        build_graph_hash: &Vec<u8>,
        shard_index: usize,
        shard_count: usize,
    ) -> anyhow::Result<()> {
        let mut items = Vec::<(NamespacedKey, Value)>::new();
        for key in keys {
            let (key, value) = self
                .read_or_fault(&key.underlying, None)
                .with_context(|| format!("Reading key {:?} failed", key))?;
            let value = value.ok_or_else(|| anyhow::anyhow!("Key {:?} not found", key))?;
            items.push((key, value));
        }

        let export = Export {
            shard_index,
            shard_count,
            items,
        };

        let manifest = ExportManifest { shard_count };

        store_export(self.backend.as_ref(), build_graph_hash, &manifest, &export)
            .context("Failed to upload the project cache export")
    }

    pub fn fetch(&self, build_graph_hash: &Vec<u8>) -> anyhow::Result<()> {
        // TODO: Expensive in terms of memory consumed. Figure out a better transaction / streaming strategy later.
        // TODO: We decode something to just encode it, which is wasteful. Fix that.
        let exports =
            fetch_exports(self.backend.as_ref(), build_graph_hash).with_context(|| {
                anyhow::anyhow!(
                    "Fetching project cache data for build graph @ {} failed",
                    hex::encode(build_graph_hash)
                )
            })?;

        let mut batch = WriteBatch::default();
        // Write exports into the batch
        for export in exports.into_iter() {
            for (key, value) in export.items {
                let key_str: String = key.try_into()?;
                let serialized_value = serde_json::to_vec(&value).with_context(|| {
                    format!(
                        "Serializing  value {:?} for key '{}' failed",
                        &value, &key_str
                    )
                })?;
                batch.put(key_str.as_bytes(), serialized_value);
            }
        }
        // Add receipt
        let receipt_key: String = {
            let key = self.import_receipt_key(build_graph_hash);
            key.try_into()?
        };
        self.database
            .put(receipt_key, IMPORT_RECEIPT_IOTA_SERIALIZED.as_slice())
            .map_err(anyhow::Error::new)?;

        // Write the batch
        self.database.write(batch).with_context(|| {
            format!(
                "Writing data for build graph state '{}' failed",
                &hex::encode(build_graph_hash)
            )
        })?;

        Ok(())
    }

    fn import_receipt_key(&self, build_graph_hash: &Vec<u8>) -> NamespacedKey {
        NamespacedKey {
            repository: self.identifier.clone(),
            underlying: Key::ImportReceipt {
                build_graph_hash: build_graph_hash.to_owned(),
            },
            version: PROJECT_CACHE_VERSION,
        }
    }

    /// Determine if the given build graph hash is marked as having been imported
    pub fn is_imported(&self, build_graph_hash: &Vec<u8>) -> anyhow::Result<bool> {
        let key_str: String = self.import_receipt_key(build_graph_hash).try_into()?;
        Ok(self.database.get_pinned(&key_str)?.is_some())
    }

    pub fn make_backend(endpoint: &Url) -> anyhow::Result<Box<dyn ProjectCacheBackend>> {
        if endpoint.scheme().eq_ignore_ascii_case("file") {
            Ok(Box::new(LocalCacheBackend::new(endpoint.clone())?))
        } else {
            Ok(Box::new(HttpCacheBackend::new(endpoint.clone())))
        }
    }
}
