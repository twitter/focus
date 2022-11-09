// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use std::{fs::File, io::Read, path::Path};

use sha2::{Digest, Sha256};

const BUFFER_SIZE: usize = 4096;

// Hash a file using SHA-256
pub fn hash(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let mut hasher = Sha256::new();
    let mut buffer: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
    let mut file = File::open(path)
        .context("Hashing file")
        .with_context(|| format!("Opening {} failed", path.display()))?;
    loop {
        let read_bytes = file.read(&mut buffer[..]).context("Read failed")?;
        if read_bytes == 0 {
            break;
        }
        hasher.update(&buffer[0..read_bytes]);
    }

    Ok(hasher.finalize().to_vec())
}
