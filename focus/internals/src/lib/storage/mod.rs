// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use rocksdb::{DBCompressionType, Options, DB};
use std::{path::Path, time::Duration};

pub fn open_database(path: impl AsRef<Path>, ttl: Duration) -> anyhow::Result<DB> {
    let path = path.as_ref();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    // Compression settings from https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning#compression
    opts.set_compression_type(DBCompressionType::Lz4);
    opts.set_bottommost_compression_type(DBCompressionType::Zstd);
    DB::open_with_ttl(&opts, path, ttl)
        .with_context(|| format!("Opening database at {}", path.display()))
        .map_err(|e| anyhow::anyhow!(e))
}
