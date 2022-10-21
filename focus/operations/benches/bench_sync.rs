// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use focus_util::app::App;

pub fn bench_sync(c: &mut Criterion) {
    let app = Arc::new(App::new_for_testing().unwrap());
    let repo_path = std::env::var_os("REPO")
        .map(PathBuf::from)
        .expect("Must set env var REPO=/path/to/repo");

    println!("Warming up with initial sync");
    focus_operations::sync::run(
        &repo_path,
        focus_operations::sync::SyncMode::Incremental,
        app.clone(),
    )
    .unwrap();

    let mut group = c.benchmark_group("noop_sync");
    group.sample_size(20);
    group.bench_function("focus_operations::sync::run", |b| {
        b.iter(|| {
            focus_operations::sync::run(
                &repo_path,
                focus_operations::sync::SyncMode::Incremental,
                app.clone(),
            )
            .unwrap()
        })
    });
}

criterion_group!(benches, bench_sync);
criterion_main!(benches);
