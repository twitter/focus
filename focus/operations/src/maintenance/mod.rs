// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod launchd;
pub mod scheduling;

use std::{
    collections::HashMap,
    ffi::OsString,
    fmt::Debug,
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use content_addressed_cache::RocksDBCache;
use focus_internals::{index::RocksDBMemoizationCacheExt, locking, tracker::Tracker};

use anyhow::{bail, Context, Result};
use focus_util::git_helper::{git_command_with_git_binary, GitBinary};
use focus_util::{app::App, git_helper::ConfigExt, sandbox_command::SandboxCommandOutput};
use maplit::hashmap;
use strum_macros;
use tracing::{debug, error, info, warn};

pub use self::launchd::{schedule_disable, schedule_enable, Launchctl, ScheduleOpts};

pub(crate) const DEFAULT_FOCUS_PATH: &str = "/opt/twitter_mde/bin/focus";
pub const DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS: &str = "/opt/twitter_mde/bin/git";

#[allow(dead_code)]
#[derive(
    Debug,
    Copy,
    Clone,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
    strum_macros::EnumIter,
)]
#[strum(serialize_all = "kebab-case")]
pub enum TimePeriod {
    Hourly,
    Daily,
    Weekly,
}

impl TimePeriod {
    pub fn name(&self) -> &'static str {
        self.into()
    }
}

pub const DEFAULT_CONFIG_KEY: &str = "maintenance.repo";

const CONFIG_DEFAULTS: &[(&str, &str)] = &[
    ("maintenance.auto", "false"),
    ("maintenance.strategy", "incremental"),
    ("maintenance.gc.enabled", "false"),
    ("maintenance.commit-graph.enabled", "false"),
    ("maintenance.prefetch.enabled", "true"),
    ("maintenance.loose-objects.enabled", "true"),
    ("maintenance.incremental-repack.enabled", "true"),
    ("log.excludedecoration", "refs/prefetch/"),
];

/// Configures the repo at `path` to have git-maintenance run the standard jobs
fn set_default_repo_config(config: &mut git2::Config) -> Result<()> {
    let mut config = config.open_level(git2::ConfigLevel::Local)?;

    for (k, v) in CONFIG_DEFAULTS.iter() {
        config.set_str_if_not_set(*k, *v)?;
    }

    Ok(())
}

// entry point from main
#[tracing::instrument]
pub fn set_default_git_maintenance_config(path: &Path) -> Result<()> {
    let repo = git2::Repository::open(path)?;
    let mut config = repo.config()?;
    set_default_repo_config(&mut config)
}

#[derive(Debug, Clone)]
pub struct RegisterOpts {
    /// The repo path to register for maintenance. Default config values will be set
    /// in the repo. If None then we assume we're operating on the "current' repo and
    /// will add the workdir path (for a plain repo) or git dir (for a bare repo) to
    /// the global config key.
    pub repo_path: Option<PathBuf>,
    /// The config key to add `repo_path` to in the global config.
    pub git_config_key: String,
    /// the path to use for the global git config. If None then use libgit2's
    /// Config::
    pub global_config_path: Option<PathBuf>,
}

impl Default for RegisterOpts {
    fn default() -> Self {
        Self {
            repo_path: None,
            git_config_key: DEFAULT_CONFIG_KEY.to_owned(),
            global_config_path: None,
        }
    }
}

/// Registers the current repository to be maintained when the maintenance runner executes
pub fn register(opts: RegisterOpts) -> Result<()> {
    debug!(?opts, "maintenance.register");
    let RegisterOpts {
        repo_path,
        git_config_key: config_key,
        global_config_path,
    } = opts;

    let repo = git2::Repository::discover(match repo_path {
        Some(rp) => rp,
        None => std::env::current_dir()?,
    })?;

    let value_for_global_config = {
        let value = repo.workdir().unwrap_or_else(|| repo.path());

        value
            .to_str()
            .unwrap_or_else(|| panic!("path was not a valid UTF-8 string: {:?}", value))
            .to_owned()
    };

    let config = use_config_path_or_default_global(global_config_path.as_deref())?;

    config.open_level(git2::ConfigLevel::Global)?.set_multivar(
        &config_key,
        &regex_escape(&value_for_global_config),
        &value_for_global_config,
    )?;

    let mut config = repo.config()?;

    set_default_repo_config(&mut config)?;

    Ok(())
}

fn use_config_path_or_default_global(config_opt: Option<&Path>) -> Result<git2::Config> {
    match config_opt {
        Some(path) => {
            let mut cfg = git2::Config::new()?;
            cfg.add_file(path, git2::ConfigLevel::Global, true)?;
            Ok(cfg)
        }
        None => {
            let mut default_config = git2::Config::open_default()?;
            Ok(default_config.open_global()?)
        }
    }
}

fn does_repo_exist(path: &Path) -> Result<bool> {
    match git2::Repository::discover(path) {
        Err(git_err) if git_err.code() == git2::ErrorCode::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(true),
    }
}

fn maint_exit_status_metric_helper(exit_status: ExitStatus) -> String {
    match exit_status.code() {
        Some(code) => code.to_string(),
        None => "signal_terminated".to_string(),
    }
}

fn sync_status_metric_helper(sync_status: crate::sync::SyncStatus) -> String {
    match sync_status {
        crate::sync::SyncStatus::Success => "success".to_string(),
        crate::sync::SyncStatus::SkippedSyncPointUnchanged => "skipped_unchanged".to_string(),
        crate::sync::SyncStatus::SkippedSyncPointDifferenceIrrelevant => {
            "skipped_irrelevant".to_string()
        }
        crate::sync::SyncStatus::SkippedPreemptiveSyncDisabled => "skipped_disabled".to_string(),
        crate::sync::SyncStatus::SkippedPreemptiveSyncCancelledByActivity => {
            "skipped_activity".to_string()
        }
        crate::sync::SyncStatus::SkippedUnfilterView => "skipped_unfiltered".to_string(),
    }
}

#[derive(Debug)]
enum MaintResult {
    Success(ExitStatus),
    LockFailed,
}

pub struct Runner<'a> {
    pub git_binary: GitBinary,
    /// the config key in the global git config that contains the list of paths to check.
    /// By default this is "maintenance.repo", a multi value key.
    pub config_key: String,
    pub config: git2::Config,
    pub tracker: &'a Tracker,
    /// if true, use the focus Tracker to discover repos
    pub tracked_repos: bool,
    pub app: Arc<App>,
}

impl Debug for Runner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("git_binary", &self.git_binary)
            .field("config_key", &self.config_key)
            .finish_non_exhaustive()
    }
}

impl Runner<'_> {
    pub fn new(opts: RunOptions, tracker: &Tracker, app: Arc<App>) -> Result<Runner> {
        let RunOptions {
            git_binary,
            git_config_key: config_key,
            git_config_path: config_path,
            tracked,
        } = opts;

        let git_binary = match git_binary {
            Some(git_binary) => git_binary,
            None => GitBinary::from_env()?,
        };

        Ok(Runner {
            git_binary,
            config_key,
            config: use_config_path_or_default_global(config_path.as_deref())?,
            tracker,
            tracked_repos: tracked,
            app,
        })
    }

    #[tracing::instrument]
    fn run_git_maint(&self, time_period: TimePeriod, repo_path: &Path) -> Result<MaintResult> {
        let (mut cmd, scmd) = git_command_with_git_binary(self.app.clone(), &self.git_binary)?;

        // TODO: this needs to log and capture output for debugging if necessary
        Ok(MaintResult::Success(
            scmd.ensure_success_or_log(
                cmd.arg("maintenance")
                    .arg("run")
                    .arg(format!("--schedule={}", time_period.name()))
                    .current_dir(repo_path),
                SandboxCommandOutput::Stderr,
            )?,
        ))
    }

    #[tracing::instrument]
    // Opening a rocksDB connection should run a compaction, if it's necessary.
    fn run_rocksdb_compaction(&self, repo_path: &Path) -> Result<()> {
        let repo = git2::Repository::open(repo_path).context("opening repo")?;
        RocksDBCache::new(&repo);
        Ok(())
    }

    #[tracing::instrument]
    fn run_internal_maint(
        &self,
        time_period: TimePeriod,
        repo_path: &Path,
    ) -> Result<crate::sync::SyncStatus> {
        let sync_result = crate::sync::run(
            repo_path,
            crate::sync::SyncMode::Preemptive { force: false },
            self.app.clone(),
        )
        .with_context(|| format!("Preemptively syncing in {}", repo_path.display()))?;

        self.run_rocksdb_compaction(repo_path)?;

        Ok(sync_result.status)
    }

    #[tracing::instrument]
    fn run_maint(&self, time_period: TimePeriod, repo_path: &Path) -> Result<MaintResult> {
        let _lock = match locking::hold_lock(repo_path, Path::new("maint.lock")) {
            Ok(lock) => lock,
            Err(e) => {
                error!(?e, "failed to acquire lock");
                return Ok(MaintResult::LockFailed);
            }
        };

        let git_maint_started_at = Instant::now();
        let git_maint_result = self
            .run_git_maint(time_period, repo_path)
            .with_context(|| format!("Running internal maintenance in {}", repo_path.display()))?;
        let git_maint_runtime = git_maint_started_at.elapsed();

        let sync_maint_started_at = Instant::now();
        let sync_result = self
            .run_internal_maint(time_period, repo_path)
            .with_context(|| format!("Running internal maintenance in {}", repo_path.display()))?;
        let sync_maint_runtime = sync_maint_started_at.elapsed();

        self.add_maint_ti_invocation_message(
            &git_maint_result,
            git_maint_runtime,
            sync_result,
            sync_maint_runtime,
        );

        Ok(git_maint_result)
    }

    #[tracing::instrument]
    fn handle_missing_config_entry(&mut self, bad_entry: &str) -> Result<()> {
        self.config
            .remove_multivar(&self.config_key, &regex_escape(bad_entry))?;
        Ok(())
    }

    fn run_tracked_repo_repair(&self) -> Result<()> {
        if !self.tracked_repos {
            return Ok(());
        }
        self.tracker.repair(self.app.clone())
    }

    fn get_repo_paths_from_config(&self) -> Result<Vec<PathBuf>> {
        Ok(self
            .config
            .multivar_values(&self.config_key, None)
            .with_context(|| {
                format!(
                    "Failed reading values for config key '{}'",
                    &self.config_key
                )
            })?
            .into_iter()
            .map(PathBuf::from)
            .collect())
    }

    fn get_repo_paths_from_tracker(&self) -> Result<Vec<PathBuf>> {
        let snapshot = self.tracker.scan().context("scanning repositories")?;

        let repos: Vec<PathBuf> = snapshot
            .repos()
            .iter()
            .map(|repo| repo.location().to_path_buf())
            .collect();

        Ok(repos)
    }

    fn get_repo_paths(&self) -> Result<Vec<PathBuf>> {
        if self.tracked_repos {
            self.get_repo_paths_from_tracker()
        } else {
            self.get_repo_paths_from_config()
        }
    }

    fn run_in_path(&self, time_period: TimePeriod, path: &Path) -> Result<()> {
        info!(?time_period, ?path, "running tasks");
        set_default_git_maintenance_config(path)?;

        let maint_result = match self.run_maint(time_period, path) {
            Ok(MaintResult::Success(status)) => {
                if status.success() {
                    debug!(?time_period, ?path, "completed maintenance",);
                    None
                } else {
                    warn!(?path, exit_status = ?status, "maintenance failed");
                    Some(maint_exit_status_metric_helper(status))
                }
            }
            Ok(MaintResult::LockFailed) => {
                warn!(?path, "failed to acquire lock");
                Some("lock_failed".to_string())
            }
            Err(e) => {
                warn!(?path, ?e, "failed running git-maintenance");
                Some("git_error".to_string())
            }
        };

        if let Some(maint_result) = maint_result {
            self.add_ti_invocation_message(&hashmap! { "maint_result".to_string() => maint_result })
        };
        Ok(())
    }

    fn add_ti_invocation_message(&self, maint_custom_map: &HashMap<String, String>) {
        let ti_client = self.app.tool_insights_client();
        ti_client.get_inner().add_invocation_message(
            SystemTime::now(),
            None,
            Some(maint_custom_map),
        );
    }

    fn add_maint_ti_invocation_message(
        &self,
        git_maint_result: &MaintResult,
        git_maint_runtime: Duration,
        sync_maint_result: crate::sync::SyncStatus,
        sync_maint_runtime: Duration,
    ) {
        self.add_ti_invocation_message(&hashmap! { "maint_result".to_string() => match git_maint_result {
                MaintResult::Success(status) => maint_exit_status_metric_helper(*status),
                MaintResult::LockFailed => "lock_failed".to_string(),
            },
            "sync_return_status".to_string() => sync_status_metric_helper(sync_maint_result),
            "git_maint_duration_sec".to_string() => git_maint_runtime.as_secs_f32().to_string(),
            "sync_maint_duration_sec".to_string() => sync_maint_runtime.as_secs_f32().to_string()
        });
    }

    #[tracing::instrument]
    pub fn run(&mut self, time_period: TimePeriod) -> Result<()> {
        if self.tracked_repos {
            self.run_tracked_repo_repair()?;
        }
        let repo_paths = self.get_repo_paths()?;

        for path in repo_paths {
            let p: &Path = &path;
            match does_repo_exist(p) {
                Ok(true) => self.run_in_path(time_period, p)?,
                Ok(false) if self.tracked_repos => {
                    info!(path=?p, "repo at returned path did not exist, continuing");
                }
                Ok(false) => self.handle_missing_config_entry(match path.to_str() {
                    Some(s) => s,
                    None => bail!("path contains invalid UTF-8: {:?}", path.to_string_lossy()),
                })?,
                Err(e) => {
                    error!(?path, ?e, "error in determining if path is a repo");
                }
            }
        }

        Ok(())
    }
}

// lets us test the construction of the Maintenance instance
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub git_binary: Option<GitBinary>,
    pub git_config_key: String,
    pub git_config_path: Option<PathBuf>,
    pub tracked: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            git_binary: None,
            git_config_key: DEFAULT_CONFIG_KEY.to_owned(),
            git_config_path: None,
            tracked: false,
        }
    }
}

pub(crate) fn regex_escape<S: AsRef<str>>(s: S) -> String {
    format!("^{}$", regex::escape(s.as_ref()))
}

#[tracing::instrument]
pub fn run(
    cli: RunOptions,
    time_period: TimePeriod,
    tracker: &Tracker,
    app: Arc<App>,
) -> Result<()> {
    Runner::new(cli, tracker, app)?.run(time_period)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use focus_testing::ScratchGitRepo;
    use tempfile::TempDir;

    struct ConfigFixture {
        pub tempdir: TempDir,
        pub config_path: PathBuf,
        pub app: Arc<App>,
    }

    impl ConfigFixture {
        fn new() -> Result<ConfigFixture> {
            let tempdir = TempDir::new()?;
            let path = tempdir.path().join("global.config");
            {
                std::fs::File::create(&path)?;
            }
            Ok(ConfigFixture {
                tempdir,
                config_path: path,
                app: Arc::new(App::new_for_testing()?),
            })
        }

        fn config(&self) -> Result<git2::Config> {
            let mut config = git2::Config::new()?;
            config.add_file(&self.config_path, git2::ConfigLevel::Global, true)?;
            Ok(config)
        }
    }

    #[test]
    fn test_handle_missing_config_entry() -> Result<()> {
        let fix = ConfigFixture::new()?;
        let tracker = Tracker::for_testing()?;

        {
            let mut config = fix.config()?;

            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/foo$", "/path/to/foo")?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/bar$", "/path/to/bar")?;

            let mut maint = Runner::new(
                RunOptions {
                    git_config_path: Some(fix.config_path.to_owned()),
                    ..Default::default()
                },
                &tracker,
                fix.app.clone(),
            )?;

            maint.handle_missing_config_entry("/path/to/foo")?;
        }

        {
            let configs = fix.config()?.multivar_values(DEFAULT_CONFIG_KEY, None)?;
            assert_eq!(configs.len(), 1);
            assert_eq!(configs[0], "/path/to/bar");
        }

        Ok(())
    }

    #[test]
    fn test_get_repo_paths() -> Result<()> {
        let fix = ConfigFixture::new()?;
        let tracker = Tracker::for_testing()?;

        {
            let mut config = fix.config()?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/foo$", "/path/to/foo")?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/bar$", "/path/to/bar")?;
        }

        {
            let maint = Runner::new(
                RunOptions {
                    git_config_path: Some(fix.config_path),
                    ..Default::default()
                },
                &tracker,
                fix.app,
            )?;

            let paths = maint.get_repo_paths()?;
            assert_eq!(
                paths,
                vec![PathBuf::from("/path/to/foo"), PathBuf::from("/path/to/bar")]
            )
        }

        Ok(())
    }

    fn assert_repo_defaults_set(config: &git2::Config) {
        for (k, v) in CONFIG_DEFAULTS.iter() {
            let val = config.get_string(k).unwrap();
            assert_eq!(
                val, *v,
                "values for key {} were not equal: {} != {}",
                *k, val, *v
            );
        }
    }

    #[test]
    fn test_set_defaults() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;
        let repo = fix.repo()?;
        let mut config = repo.config()?;

        set_default_repo_config(&mut config)?;
        assert_repo_defaults_set(&config);

        Ok(())
    }

    #[test]
    fn test_try_from_cli_options() -> Result<()> {
        let fix = ConfigFixture::new()?;
        let tracker = Tracker::for_testing()?;
        {
            let mut conf = fix.config()?;
            conf.set_bool("testing.testing.onetwothree", true)?;
        }

        let git_binary = GitBinary {
            home_temp_dir: None,
            git_binary_path: "/path/to/bin/git".into(),
            git_exec_path: "/path/to/lib/gitcore".into(),
            env: Default::default(),
        };
        let config_key = "other.key";
        let config_path = fix.config_path;

        let opts = RunOptions {
            git_binary: Some(git_binary.clone()),
            git_config_key: config_key.into(),
            git_config_path: Some(config_path),
            tracked: false,
        };

        let runner = Runner::new(opts, &tracker, fix.app)?;

        assert_eq!(runner.git_binary, git_binary);
        assert_eq!(runner.config_key, config_key.to_string());

        let conf = &runner.config;
        assert!(conf.get_bool("testing.testing.onetwothree")?);

        Ok(())
    }

    #[test]
    fn test_register() -> Result<()> {
        let fix = ConfigFixture::new()?;
        let scratch = ScratchGitRepo::new_static_fixture(fix.tempdir.path())?;
        let repo = scratch.repo()?;

        register(RegisterOpts {
            repo_path: Some(scratch.repo()?.workdir().unwrap().to_owned()),
            global_config_path: Some(fix.config_path.clone()),
            ..RegisterOpts::default()
        })?;

        {
            let config = repo.config()?.open_level(git2::ConfigLevel::Local)?;
            assert_repo_defaults_set(&config);
        }

        let global_config = fix.config()?;

        let values = global_config.multivar_values(DEFAULT_CONFIG_KEY, None)?;
        assert!(!values.is_empty());

        Ok(())
    }
}
