use serde::{Deserialize, Serialize};

/// A project is a collection of targets.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexConfig {
    /// Whether fetching is enabled
    pub enabled: bool,

    /// Remote URL
    pub remote: String,
}
