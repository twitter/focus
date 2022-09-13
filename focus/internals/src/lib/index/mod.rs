// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Focus dependency graph index implementation.
//!
//! See <http://go/focus-index> for design notes.

#![warn(missing_docs)]

mod churn;
mod content_hash;
mod dependency_graph;
mod object_database;

pub use churn::print_churn_stats;
pub use content_hash::{content_hash, ContentHash, HashContext};
pub use dependency_graph::{
    get_files_to_materialize, update_object_database_from_resolution, DependencyKey,
    DependencyValue, PathsToMaterializeResult,
};
pub use object_database::{
    ObjectDatabase, RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb, FUNCTION_ID,
};

#[cfg(test)]
pub use object_database::testing;
