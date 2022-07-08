//! Run with
//!
//! ```bash
//! cargo run --example calc_invalidation_rate -- ~/workspace/path/to/repo 10000
//! ```

use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use clap::Parser;
use focus_internals::{
    index::{content_hash_dependency_key, ContentHash, DependencyKey, HashContext},
    model::repo::Repo,
    target::{Target, TargetSet},
};
use focus_util::{app::App, git_helper};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

#[derive(Parser, Debug)]
struct Opts {
    sparse_repo_path: PathBuf,

    /// The number of commits backward from the `HEAD` commit to sample.
    num_commits: usize,
}

struct RepoPool {
    repo_path: PathBuf,
    repos: Arc<Mutex<Vec<git2::Repository>>>,
}

struct RepoHandle {
    repo: Option<git2::Repository>,
    repos: Arc<Mutex<Vec<git2::Repository>>>,
}

impl Deref for RepoHandle {
    type Target = git2::Repository;

    fn deref(&self) -> &Self::Target {
        self.repo.as_ref().unwrap()
    }
}

impl Drop for RepoHandle {
    fn drop(&mut self) {
        let mut repos = self.repos.lock().unwrap();
        repos.push(self.repo.take().unwrap());
    }
}

impl RepoPool {
    fn new(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            repos: Default::default(),
        }
    }

    fn get(&self) -> anyhow::Result<RepoHandle> {
        let mut repos = self.repos.lock().unwrap();
        let repo = match repos.pop() {
            Some(repo) => repo,
            None => git2::Repository::open(&self.repo_path)?,
        };
        Ok(RepoHandle {
            repo: Some(repo),
            repos: Arc::clone(&self.repos),
        })
    }
}

fn average(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut sum: f64 = 0.0;
    let mut len: f64 = 0.0;
    for value in values {
        sum += value as f64;
        len += 1.0;
    }
    sum / len
}

fn main() -> anyhow::Result<()> {
    let Opts {
        sparse_repo_path,
        num_commits,
    } = Opts::parse();

    let app = Arc::new(App::new(false, None, None, None)?);
    let repo = Repo::open(&sparse_repo_path, app)?;
    let selections = repo.selection_manager()?;
    let all_targets = {
        let mut targets = TargetSet::try_from(&selections.project_catalog().mandatory_projects)?;
        targets.extend(TargetSet::try_from(
            &selections.project_catalog().optional_projects,
        )?);
        targets
    };

    let repo = git2::Repository::open(sparse_repo_path.clone())?;
    let commits = {
        eprintln!("Loading {num_commits} commits...");
        let mut commit = git_helper::get_head_commit(&repo)?;
        let mut result = Vec::new();
        for i in 0..num_commits {
            result.push(commit.id());
            commit = match commit.parents().next() {
                Some(commit) => commit,
                None => {
                    eprintln!("Stopped after {i} commits.");
                    break;
                }
            };
        }
        result
    };

    let repo_pool = RepoPool::new(sparse_repo_path);
    let num_finished_commits = AtomicUsize::new(0);
    let hashes: HashMap<(git2::Oid, &Target), ContentHash> = commits
        .par_iter()
        .flat_map(|commit_oid| {
            let repo = repo_pool.get().unwrap();
            let commit = repo.find_commit(*commit_oid).unwrap();
            eprintln!(
                "Hashing {}/{num_commits}: {:?}",
                num_finished_commits.fetch_add(1, Ordering::SeqCst),
                commit.summary().unwrap()
            );
            let tree = commit.tree().unwrap();
            let hash_context = HashContext {
                repo: &repo,
                head_tree: &tree,
                caches: Default::default(),
            };
            all_targets
                .iter()
                .map(|target| {
                    let dep_key = DependencyKey::from(target.clone());
                    let hash =
                        content_hash_dependency_key(&hash_context, &dep_key, &mut HashSet::new())
                            .unwrap();
                    ((*commit_oid, target), hash)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    #[derive(Clone, Debug)]
    struct HashChangeInfo {
        current_hash: ContentHash,
        commits_since_last_hash_change: usize,
    }
    let mut target_stats: HashMap<&Target, Vec<HashChangeInfo>> = all_targets
        .iter()
        .map(|target| (target, Vec::new()))
        .collect();

    for commit_oid in &commits {
        for (target, hash_change_infos) in target_stats.iter_mut() {
            let hash = hashes[&(*commit_oid, *target)].clone();
            match hash_change_infos.last_mut() {
                Some(hash_change_info) if hash_change_info.current_hash == hash => {
                    hash_change_info.commits_since_last_hash_change += 1;
                }
                _ => {
                    hash_change_infos.push(HashChangeInfo {
                        current_hash: hash,
                        commits_since_last_hash_change: 0,
                    });
                }
            }
        }
    }

    let mut averages: Vec<_> = target_stats
        .iter()
        .map(|(target, hash_change_infos)| {
            let changes: Vec<_> = hash_change_infos
                .iter()
                .map(|hash_change_info| hash_change_info.commits_since_last_hash_change as f64)
                .collect();
            (target, average(changes))
        })
        .collect();
    averages.sort_by(|(_, lhs), (_, rhs)| lhs.partial_cmp(rhs).unwrap());
    println!("Most churning targets:");
    for (target, value) in &averages[..10] {
        println!("{target} {value:.2}");
    }

    let average_churn = average(averages.iter().map(|(_, value)| *value));
    println!("Average churn rate: {average_churn:.2}");

    Ok(())
}
