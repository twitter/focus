//! Focus dependency graph index implementation.
//!
//! See <http://go/focus-index> for design notes.

#![warn(missing_docs)]

mod content_hash;
mod dependency_graph;
mod object_database;

pub use content_hash::{content_hash_dependency_key, ContentHash, HashContext};
pub use dependency_graph::{
    get_files_to_materialize, update_object_database_from_resolution, DependencyKey,
    DependencyValue, PathsToMaterializeResult,
};
pub use object_database::testing;
pub use object_database::{
    ObjectDatabase, RocksDBCache, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
