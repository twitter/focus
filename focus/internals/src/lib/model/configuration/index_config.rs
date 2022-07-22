// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexConfig {
    /// Whether fetching is enabled
    pub enabled: bool,

    /// Remote URL
    pub remote: String,
}
