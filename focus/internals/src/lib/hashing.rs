// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

type Hasher = Sha256;

/// Hash a file's lines without line separators
pub fn hash_file(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Opening {} for hashing", path.display()))?;

    let mut digest = Hasher::new();
    digest.update(&contents);
    Ok(digest.finalize().to_vec())
}
