// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use vergen::{vergen, Config};

fn main() {
    vergen(Config::default()).unwrap()
}
