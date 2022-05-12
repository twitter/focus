use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

type Hasher = Sha256;

/// Hash a file's lines without line separators
pub fn hash_file_lines(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let buffered_reader = BufReader::new(
        File::open(&path).with_context(|| format!("Opening {} for hashing", path.display()))?,
    );

    let mut digest = Hasher::new();
    for line in buffered_reader.lines() {
        let line = line.context("Failed to read line")?;
        digest.update(&line);
    }

    Ok(digest.finalize().to_vec())
}
