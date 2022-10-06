// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;

use serde::{Deserialize, Serialize};
use std::{fmt::Display, collections::BTreeMap};
use url::Url;

use crate::model::outlining::PatternSet;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord)]
pub struct RepoIdentifier {
    pub(crate) host: String,
    pub(crate) name: String,
}

impl RepoIdentifier {
    pub fn from(repository: &git2::Repository) -> anyhow::Result<RepoIdentifier> {
        // TODO: Fix this to not only work with the 'origin' remote.
        let remote = repository
            .find_remote("origin")
            .context("Resolving origin remote")?;
        let url = remote
            .url()
            .ok_or_else(|| anyhow::anyhow!("Origin remote has no URL"))?;
        let url = Url::parse(url)
            .with_context(|| format!("Could not parse origin URL from '{}'", url))?;
        RepoIdentifier::try_from(url)
    }
}

impl TryFrom<Url> for RepoIdentifier {
    type Error = anyhow::Error;

    fn try_from(value: Url) -> Result<Self, Self::Error> {
        let host = value
            .host_str()
            .unwrap_or_else(|| {
                if value.scheme().eq("file") {
                    "localhost"
                } else {
                    "unknown"
                }
            })
            .to_owned();
        let host = host;
        let name = value.path();
        let name = name.strip_prefix('/').unwrap_or(name); // Strip leading '/'
        let name = name.strip_suffix(".git").unwrap_or(name).to_string(); // Strip trailing '.git'

        Ok(Self { host, name })
    }
}

impl Display for RepoIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.host, self.name)
    }
}

/// Manifests store how many shards were used when an export was created so that clients can know what to fetch.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportManifest {
    pub(crate) shard_count: usize,
    pub(crate) mandatory_items: BTreeMap<NamespacedKey, Value>,
}

/// Container for keys and values,
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Export {
    pub(crate) shard_index: usize,
    pub(crate) shard_count: usize,
    pub(crate) items: BTreeMap<NamespacedKey, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    CommitToBuildGraphHash {
        #[serde(with = "hex::serde")]
        commit_id: Vec<u8>,
    },
    MandatoryProjectPatternSet {
        #[serde(with = "hex::serde")]
        build_graph_hash: Vec<u8>,
    },
    OptionalProjectPatternSet {
        #[serde(with = "hex::serde")]
        build_graph_hash: Vec<u8>,
        project_name: String,
    },
    ImportReceipt {
        build_graph_hash: Vec<u8>,
    },
}

impl Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::CommitToBuildGraphHash { commit_id } => {
                write!(
                    f,
                    "commit-to-build-graph-hash:commit={}",
                    hex::encode(commit_id)
                )
            }
            Key::MandatoryProjectPatternSet { build_graph_hash } => {
                write!(
                    f,
                    "mandatory-project-pattern-set:build-graph-hash={}",
                    hex::encode(build_graph_hash),
                )
            },
            Key::OptionalProjectPatternSet {
                build_graph_hash,
                project_name,
            } => {
                write!(
                    f,
                    "optional-project-pattern-set:build-graph-hash={}:project={}",
                    hex::encode(build_graph_hash),
                    project_name
                )
            }
            Key::ImportReceipt { build_graph_hash } => {
                write!(
                    f,
                    "import-reciept:build-graph-hash={}",
                    hex::encode(build_graph_hash),
                )
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    BuildGraphHash {
        #[serde(with = "hex::serde")]
        build_graph_hash: Vec<u8>,
    },
    MandatoryProjectPatternSet(PatternSet),
    OptionalProjectPatternSet(PatternSet),
    ImportReceiptIota,
}

/// Namespaced project cache keys act as an envelope identifying the repository cached content corresponds to.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord)]
pub struct NamespacedKey {
    pub(crate) repository: RepoIdentifier,
    pub(crate) underlying: Key,
    pub(crate) version: usize,
}
// TODO: Consider removing these namespaced keys because they are now redundant

impl TryInto<String> for NamespacedKey {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<String, Self::Error> {
        Ok(format!(
            "{};{}{}",
            self.repository,
            self.underlying,
            super::VERSION_KEY_SUFFIX.as_str()
        ))
    }
}

#[cfg(test)]
mod testing {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_repository_identifier_from_url() {
        assert_eq!(
            RepoIdentifier::try_from(Url::from_str("https://github.com/twitter/focus").unwrap())
                .unwrap(),
            RepoIdentifier {
                host: String::from("github.com"),
                name: String::from("twitter/focus"),
            }
        );

        assert_eq!(
            RepoIdentifier::try_from(
                Url::from_str("https://github.com/twitter/focus.git").unwrap()
            )
            .unwrap(),
            RepoIdentifier {
                host: String::from("github.com"),
                name: String::from("twitter/focus"),
            }
        );

        assert_eq!(
            RepoIdentifier::try_from(
                Url::from_str("https://github.com/twitter/focus.git").unwrap()
            )
            .unwrap(),
            RepoIdentifier {
                host: String::from("github.com"),
                name: String::from("twitter/focus"),
            }
        );

        assert_eq!(
            RepoIdentifier::try_from(Url::from_str("file:///home/alice/code/focus.git").unwrap())
                .unwrap(),
            RepoIdentifier {
                host: String::from("localhost"),
                name: String::from("home/alice/code/focus"),
            }
        );
    }
}
