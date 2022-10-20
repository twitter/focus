// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use core::fmt;
use core::sync::atomic::AtomicBool;
use focus_internals::index::RocksDBMemoizationCacheExt;
use focus_internals::{locking, model::repo::Repo};

use crate::util::perform;
use content_addressed_cache::RocksDBCache;
use focus_util::app::App;
use focus_util::backed_up_file::BackedUpFile;
use tracing::{debug, info, warn};

use std::path::Path;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;

const PREEMPTIVE_SYNC_MAX_WAIT_MILLIS: u64 = 30000;
const TEST_ONLY_PREEMPTIVE_SYNC_MAX_WAIT_MILLIS_UNDER_TEST: u64 = 300;
const PREEMPTIVE_SYNC_POLL_INTERVAL_MILLIS: u64 = 100;
pub(crate) const SYNC_FROM_PROJECT_CACHE_REQUIRED_ERROR_MESSAGE: &str =
    "Sync from project cache was required but not possible";

lazy_static! {
    static ref TEST_ONLY_PREEMPTIVE_SYNC_MACHINE_IS_ACTIVE: AtomicBool = AtomicBool::new(false);
}

pub fn test_only_get_preemptive_sync_machine_is_active() -> bool {
    use std::sync::atomic::Ordering;
    TEST_ONLY_PREEMPTIVE_SYNC_MACHINE_IS_ACTIVE.load(Ordering::SeqCst)
}

#[cfg(test)]
pub fn test_only_set_preemptive_sync_machine_is_active(new_value: bool) {
    use std::sync::atomic::Ordering;
    TEST_ONLY_PREEMPTIVE_SYNC_MACHINE_IS_ACTIVE.store(new_value, Ordering::SeqCst);
}

/// An enumeration indicating which kind of sync should be performed.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SyncMode {
    /// Perform a normal sync
    Normal,

    /// Perform a preemptive sync
    Preemptive {
        /// Whether to skip enablement and machine idleness checks
        force: bool,
    },

    // Peform a one-shot sync, not using the cache
    OneShot,

    /// Perform a sync using only data from the project cache
    RequireProjectCache,
}

/// An enumeration capturing that the sync was peformed or a reason it was skipped.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// The sync was performed.
    Success,

    /// There is no change to the sync point itself.
    SkippedSyncPointUnchanged,

    /// The content that has changed is not relevant to the build graph.
    SkippedSyncPointDifferenceIrrelevant,

    /// Preemptive syncing is not enabled.
    SkippedPreemptiveSyncDisabled,

    /// Preemptive syncing was cancelled because the machine is actively being used.
    SkippedPreemptiveSyncCancelledByActivity,
}

/// An enumeration capturing which mechanism was used to perform the sync.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncMechanism {
    /// The sync was performed via outlining, incorporating data from the content addressed cache where possible.
    CachedOutline,

    /// The sync was performed via outlining in one shot, without using the content addressed cache.
    OneShotOutline,

    /// The sync was peformed by consulting the project cache.
    ProjectCache,
}

impl fmt::Display for SyncMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncMechanism::CachedOutline => write!(f, "outline"),
            SyncMechanism::OneShotOutline => write!(f, "one-shot-outline"),
            SyncMechanism::ProjectCache => write!(f, "project-cache"),
        }
    }
}

/// State describing the outcome of a sync.
pub struct SyncResult {
    /// Whether the working tree was checked out during the sync
    pub checked_out: bool,

    /// The commit ID that was synchronized
    pub commit_id: Option<git2::Oid>,

    /// The action taken
    pub status: SyncStatus,

    /// The mechanism used to perform the sync
    pub mechanism: SyncMechanism,
}

/// Synchronize the sparse repo's contents with the build graph. Returns a SyncResult indicating what happened.
pub fn run(sparse_repo: &Path, mode: SyncMode, app: Arc<App>) -> Result<SyncResult> {
    let repo = Repo::open(sparse_repo, app.clone()).context("Failed to open the repo")?;

    let (preemptive, force) = match mode {
        SyncMode::Preemptive { force: forced } => (true, forced),
        _ => (false, false),
    };

    if preemptive && !force {
        if !repo.get_preemptive_sync_enabled()? {
            return Ok(SyncResult {
                checked_out: false,
                commit_id: None,
                status: SyncStatus::SkippedPreemptiveSyncDisabled,
                mechanism: SyncMechanism::CachedOutline,
            });
        }

        let idle_duration = repo.get_preemptive_sync_idle_threshold()?;
        let max_wait = Duration::from_millis(if cfg!(test) {
            TEST_ONLY_PREEMPTIVE_SYNC_MAX_WAIT_MILLIS_UNDER_TEST
        } else {
            PREEMPTIVE_SYNC_MAX_WAIT_MILLIS
        });
        let poll_interval = Duration::from_millis(PREEMPTIVE_SYNC_POLL_INTERVAL_MILLIS);
        info!(
            ?idle_duration,
            ?max_wait,
            ?poll_interval,
            "Waiting for machine to become idle"
        );
        if wait_for_machine_to_be_idle(idle_duration, max_wait, poll_interval)
            .context("Failed waiting for machine to be idle")?
        {
            info!("Machine is idle, continuing preemptive sync");
        } else {
            info!("Machine is busy, cancelling preemptive sync");
            return Ok(SyncResult {
                checked_out: false,
                commit_id: None,
                status: SyncStatus::SkippedPreemptiveSyncCancelledByActivity,
                mechanism: SyncMechanism::CachedOutline,
            });
        }
    }

    let _lock = locking::hold_lock(sparse_repo, Path::new("sync.lock"))
        .context("Failed to obtain synchronization lock")?;

    let sparse_profile_path = repo.git_dir().join("info").join("sparse-checkout");
    if !sparse_profile_path.is_file() {
        bail!("This does not appear to be a focused repo -- it is missing a sparse checkout file");
    }

    let selections = repo.selection_manager()?;
    let selection = selections.computed_selection()?;
    let targets = selections.compute_complete_target_set()?;

    let mut mechanism = SyncMechanism::CachedOutline;

    // Add target/project to TI data.
    let app_for_ti_client = app.clone();
    let ti_client = app_for_ti_client.tool_insights_client();
    ti_client.get_context().add_to_custom_map(
        "sync_kind",
        if preemptive {
            "preemptive"
        } else {
            "immediate"
        },
    );

    let backed_up_sparse_profile: Option<BackedUpFile> = if preemptive {
        None
    } else {
        super::ensure_clean::run(sparse_repo, app.clone())
            .context("Failed trying to determine whether working trees were clean")?;

        ti_client
            .get_context()
            .add_to_custom_map("total_target_count", targets.len().to_string());
        ti_client.get_context().add_to_custom_map(
            "user_selected_project_count",
            selection.projects.len().to_string(),
        );
        ti_client.get_context().add_to_custom_map(
            "user_selected_target_count",
            selection.targets.len().to_string(),
        );
        let mut project_selection_names: Vec<String> =
            selection.projects.iter().map(|n| n.name.clone()).collect();
        let mut target_selection_names: Vec<String> =
            selection.targets.iter().map(|n| n.to_string()).collect();
        project_selection_names.sort();
        target_selection_names.sort();
        ti_client.get_context().add_to_custom_map(
            "user_project_selection",
            serde_json::to_string(&project_selection_names)?,
        );
        ti_client.get_context().add_to_custom_map(
            "user_target_selection",
            serde_json::to_string(&target_selection_names)?,
        );

        Some(BackedUpFile::new(&sparse_profile_path)?)
    };

    let head_commit = repo.get_head_commit().context("Resolving head commit")?;

    // Figure out if this repo has a "master" branch or "main" branch.
    let primary_branch_name = repo
        .primary_branch_name()
        .context("Determining primary branch name")?;

    let commit = if preemptive {
        if let Some(prefetch_commit) = repo
            .get_prefetch_head_commit("origin", primary_branch_name.as_str())
            .context("Resolving prefetch head commit")?
        {
            prefetch_commit
        } else {
            bail!("No prefetch commit found for preemptive sync");
        }
    } else {
        head_commit
    };

    if preemptive {
        if let Some(working_tree) = repo.working_tree() {
            if let Ok(Some(sync_point)) = working_tree.read_sparse_sync_point_ref() {
                if sync_point == commit.id() {
                    // The sync point is already set to this ref. We don't need to bother.
                    warn!("Skipping preemptive synchronization because the commit to sync is the same as that of the sync point");
                    return Ok(SyncResult {
                        checked_out: false,
                        commit_id: Some(commit.id()),
                        status: SyncStatus::SkippedSyncPointUnchanged,
                        mechanism,
                    });
                }
            } else if let Ok(Some(sync_point)) = working_tree.read_preemptive_sync_point_ref() {
                if sync_point == commit.id() {
                    // The sync point is already set to this ref. We don't need to bother.
                    warn!("Skipping preemptive synchronization because the commit to sync is the same as that of the preemptive sync point");
                    return Ok(SyncResult {
                        checked_out: false,
                        commit_id: Some(commit.id()),
                        status: SyncStatus::SkippedSyncPointUnchanged,
                        mechanism,
                    });
                }
            }
        }
        // TODO: Skip outlining if there are no changes to the build graph between the last and new prospective sync point
    }

    // If only projects are selected (no ad-hoc targets) we try to use the project cache to sync. Otherwise we fall back to regular syncing.
    let (pattern_count, checked_out) = perform("Computing the new sparse profile", || {
        // Try to use the project cache
        let project_cache_result = repo
            .sync_using_project_cache(commit.id(), &selection)
            .context("Syncing from project cache failed");

        match project_cache_result {
            // Answered from project cache optionally
            Ok(Some(inner)) => {
                mechanism = SyncMechanism::ProjectCache;
                Ok(inner)
            }
            // No answer from project cache when one was required
            _ if mode == SyncMode::RequireProjectCache => Err(anyhow::anyhow!(
                SYNC_FROM_PROJECT_CACHE_REQUIRED_ERROR_MESSAGE,
            )),
            _ => {
                // Report a project cache error if one was encountered
                if project_cache_result.is_err() {
                    warn!(error = ?project_cache_result.unwrap_err(), "Project cache encounted an error");
                }

                // If one-shot Bazel resolution is explicitly requested, or is allowed by config, use it
                let one_shot = match mode {
                    SyncMode::Normal => repo.get_bazel_oneshot_resolution()?,
                    SyncMode::Preemptive { .. } => false,
                    SyncMode::OneShot => true,
                    SyncMode::RequireProjectCache => unreachable!(),
                };

                let cache: Option<RocksDBCache> = if one_shot {
                    None
                } else {
                    Some(RocksDBCache::new(repo.underlying()))
                };

                repo.sync(
                    commit.id(),
                    &targets,
                    preemptive,
                    app.clone(),
                    cache.as_ref(),
                )
                .context("Sync failed")
            }
        }
    })?;

    if preemptive {
        perform("Updating the sync point", || {
            repo.working_tree()
                .unwrap()
                .write_preemptive_sync_point_ref(commit.id())
        })?;
    } else {
        ti_client
            .get_context()
            .add_to_custom_map("pattern_count", pattern_count.to_string());
        ti_client
            .get_context()
            .add_to_custom_map("sync_mechanism", mechanism.to_string());
        perform("Updating the sync point", || {
            repo.working_tree().unwrap().write_sync_point_ref()
        })?;

        // The profile was successfully applied, so do not restore the backup.
        backed_up_sparse_profile.unwrap().set_restore(false);
    }

    Ok(SyncResult {
        checked_out,
        commit_id: Some(commit.id()),
        status: SyncStatus::Success,
        mechanism,
    })
}

/// Wait for the machine to be idle for a given time period, waiting up to some maximum, and polling at a given interval.
fn wait_for_machine_to_be_idle(
    idle_duration: Duration,
    max_wait: Duration,
    poll_interval: Duration,
) -> Result<bool> {
    use focus_platform::session_state;

    if max_wait < idle_duration {
        bail!("max_wait must be greater than idle_duration")
    } else if poll_interval > max_wait {
        bail!("poll_interval must be less than max_wait")
    }

    let started_at = SystemTime::now();
    loop {
        let elapsed = started_at
            .elapsed()
            .context("Determining elapsed time failed")?;
        if elapsed > max_wait {
            break;
        }
        let state = {
            // If we are running under test, read from a variable instead of doing any polling.
            if cfg!(test) {
                debug!("Running under test!");
                if test_only_get_preemptive_sync_machine_is_active() {
                    debug!("Pretending machine is active");
                    session_state::SessionStatus::Active
                } else {
                    debug!("Pretending machine is idle");
                    session_state::SessionStatus::Idle
                }
            } else {
                unsafe { session_state::has_session_been_idle_for(idle_duration) }
            }
        };

        match state {
            session_state::SessionStatus::Active => {
                std::thread::sleep(poll_interval);
            }
            _ => {
                // Note: If we can't determine whether the session is idle, just go ahead.
                return Ok(true);
            }
        }
    }

    Ok(false)
}
