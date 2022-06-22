// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Result};

use tracing::error;

pub fn perform<F, J>(description: &str, f: F) -> Result<J>
where
    F: FnOnce() -> Result<J>,
{
    let result = f();
    if let Err(e) = result {
        error!("Failed {}: {}", description.to_ascii_lowercase(), e);
        bail!(e);
    }

    result
}
