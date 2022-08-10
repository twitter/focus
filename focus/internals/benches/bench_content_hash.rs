// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use content_addressed_cache::RocksDBCache;
use criterion::{criterion_group, criterion_main, Criterion};
use focus_internals::index::{
    content_hash_dependency_key, ContentHash, DependencyKey, DependencyValue, HashContext,
    ObjectDatabase, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use focus_internals::model::repo::Repo;
use focus_internals::target::TargetSet;
use focus_util::app::App;

fn content_hash_dependency_keys(ctx: &HashContext, dep_keys: &[DependencyKey]) -> Vec<ContentHash> {
    dep_keys
        .iter()
        .map(|dep_key| {
            let dep_key = DependencyKey::DummyForTesting(Box::new(dep_key.clone()));
            content_hash_dependency_key(ctx, &dep_key, &mut HashSet::new()).unwrap()
        })
        .collect::<Vec<_>>()
}

pub fn bench_content_hash(c: &mut Criterion) {
    let app = Arc::new(App::new_for_testing().unwrap());
    let repo_path = std::env::var_os("REPO")
        .map(PathBuf::from)
        .expect("Must set env var REPO=/path/to/repo");

    let repo = Repo::open(&repo_path, app).unwrap();
    let git_repo = repo.underlying();
    let head_commit = repo.get_head_commit().unwrap();
    let head_tree = head_commit.tree().unwrap();
    let selections = repo.selection_manager().unwrap();
    let dep_keys = selections
        .compute_complete_target_set()
        .unwrap()
        .into_iter()
        .map(DependencyKey::from)
        .collect::<Vec<DependencyKey>>();
    println!("Dependency keys: {:?}", &dep_keys);

    c.bench_function("content_hash_mandatory_layers", |b| {
        b.iter(|| {
            let hash_context = HashContext {
                repo: git_repo,
                head_tree: &head_tree,
                caches: Default::default(),
            };
            content_hash_dependency_keys(&hash_context, &dep_keys)
        })
    });

    {
        let odb = SimpleGitOdb::new(git_repo);
        c.bench_function("content_hash_insert_simple_git_odb", |b| {
            b.iter_batched(
                || {
                    odb.clear().unwrap();
                    HashContext {
                        repo: git_repo,
                        head_tree: &head_tree,
                        caches: Default::default(),
                    }
                },
                |hash_context| {
                    for dep_key in dep_keys.iter() {
                        odb.put(
                            &hash_context,
                            dep_key,
                            DependencyValue::DummyForTesting(dep_key.clone()),
                        )
                        .unwrap();
                    }
                },
                criterion::BatchSize::SmallInput,
            )
        });
        odb.clear().unwrap();
    }

    {
        let odb = RocksDBCache::new(git_repo);
        c.bench_function("content_hash_insert_rocks_db", |b| {
            b.iter_batched(
                || {
                    odb.clear().unwrap();
                    HashContext {
                        repo: git_repo,
                        head_tree: &head_tree,
                        caches: Default::default(),
                    }
                },
                |hash_context| {
                    for dep_key in dep_keys.iter() {
                        odb.put(
                            &hash_context,
                            dep_key,
                            DependencyValue::DummyForTesting(dep_key.clone()),
                        )
                        .unwrap();
                    }
                },
                criterion::BatchSize::SmallInput,
            )
        });
        odb.clear().unwrap();
    }
}

criterion_group!(benches, bench_content_hash);
criterion_main!(benches);
