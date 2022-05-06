use std::path::PathBuf;

use content_addressed_cache::RocksDBCache;
use criterion::{criterion_group, criterion_main, Criterion};
use focus_internals::index::{
    content_hash_dependency_key, ContentHash, DependencyKey, DependencyValue, HashContext,
    ObjectDatabase, RocksDBMemoizationCacheExt, SimpleGitOdb,
};
use focus_internals::model::project::ProjectSets;
use focus_internals::target::Target;

fn content_hash_dependency_keys(ctx: &HashContext, dep_keys: &[DependencyKey]) -> Vec<ContentHash> {
    dep_keys
        .iter()
        .map(|dep_key| {
            let dep_key = DependencyKey::DummyForTesting(Box::new(dep_key.clone()));
            content_hash_dependency_key(ctx, &dep_key).unwrap()
        })
        .collect::<Vec<_>>()
}

pub fn bench_content_hash(c: &mut Criterion) {
    let repo_path = std::env::var_os("REPO").expect("Must set env var REPO=/path/to/repo");
    let repo_path = PathBuf::from(repo_path);
    let repo = git2::Repository::open(&repo_path).unwrap();
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let head_tree = head_commit.tree().unwrap();

    let mandatory_layers = ProjectSets::new(&repo_path).mandatory_projects().unwrap();
    let dep_keys: Vec<DependencyKey> = mandatory_layers
        .projects()
        .iter()
        .flat_map(|project| {
            project
                .targets()
                .iter()
                .map(|target| Target::try_from(target.as_str()).unwrap())
                .map(DependencyKey::from)
        })
        .collect();
    println!("Dependency keys: {:?}", &dep_keys);

    c.bench_function("content_hash_mandatory_layers", |b| {
        b.iter(|| {
            let hash_context = HashContext {
                repo: &repo,
                head_tree: &head_tree,
                caches: Default::default(),
            };
            content_hash_dependency_keys(&hash_context, &dep_keys)
        })
    });

    {
        let odb = SimpleGitOdb::new(&repo);
        c.bench_function("content_hash_insert_simple_git_odb", |b| {
            b.iter_batched(
                || {
                    odb.clear().unwrap();
                    HashContext {
                        repo: &repo,
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
        let odb = RocksDBCache::new(&repo);
        c.bench_function("content_hash_insert_rocks_db", |b| {
            b.iter_batched(
                || {
                    odb.clear().unwrap();
                    HashContext {
                        repo: &repo,
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
