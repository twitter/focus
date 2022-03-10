pub mod launchd;

use std::{
    ffi::OsString,
    io::{BufRead, Cursor},
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use crate::util::git_helper;
use anyhow::{Context, Result};
use strum_macros;
use tracing::{debug, error, info, warn};

pub use self::launchd::{
    schedule_disable, schedule_enable, Launchctl, PlistOpts, ScheduleOpts, DEFAULT_FOCUS_PATH,
    DEFAULT_GIT_BINARY_PATH_FOR_PLISTS,
};

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
    ("maintenance.commit-graph.enabled", "true"),
    ("maintenance.prefetch.enabled", "true"),
    ("maintenance.loose-objects.enabled", "true"),
    ("maintenance.incremental-repack.enabled", "true"),
    ("log.excludedecoration", "refs/prefetch/"),
];

/// Configures the repo at `path` to have git-maintenance run the standard jobs
#[allow(dead_code)]
fn set_default_repo_config(config: &mut git2::Config) -> Result<()> {
    let mut config = config.open_level(git2::ConfigLevel::Local)?;

    for (k, v) in CONFIG_DEFAULTS.iter() {
        config.set_str_if_not_set(*k, *v)?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct RegisterOpts {
    /// The repo path to register for maintenance. Default config values will be set
    /// in the repo. If None then we assume we're operating on the "current' repo and
    /// will add the workdir path (for a plain repo) or git dir (for a bare repo) to
    /// the global config key.
    pub repo_path: Option<PathBuf>,
    /// The config key to add `repo_path` to in the global config.
    pub config_key: String,
    /// the path to use for the global git config. If None then use libgit2's
    /// Config::
    pub global_config_path: Option<PathBuf>,
}

impl Default for RegisterOpts {
    fn default() -> Self {
        Self {
            repo_path: None,
            config_key: DEFAULT_CONFIG_KEY.to_owned(),
            global_config_path: None,
        }
    }
}

/// Registers the current repository to be maintained when the maintenance runner executes
pub fn register(opts: RegisterOpts) -> Result<()> {
    debug!(?opts, "maintenance.register");
    let RegisterOpts {
        repo_path,
        config_key,
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

pub struct Runner {
    pub git_binary_path: PathBuf,
    /// the config key in the global git config that contains the list of paths to check.
    /// By default this is "maintenance.repo", a multi value key.
    pub config_key: String,
    pub config: git2::Config,
}

impl Runner {
    pub fn new(opts: RunOptions) -> Result<Runner> {
        let RunOptions {
            git_binary_path,
            config_key,
            config_path,
        } = opts;

        let git_binary_path = if git_binary_path == Path::new(DEFAULT_GIT_BINARY_PATH) {
            git_helper::git_binary_path()?
        } else {
            git_binary_path
        };

        Ok(Runner {
            git_binary_path,
            config_key,
            config: use_config_path_or_default_global(config_path.as_deref())?,
        })
    }

    fn run_maint(&self, time_period: TimePeriod, repo_path: &Path) -> Result<Output> {
        let exec_path: PathBuf = git_helper::git_exec_path(&self.git_binary_path)?;

        // TODO: this needs to log and capture output for debugging if necessary
        Command::new(&self.git_binary_path)
            .arg({
                let mut s = OsString::new();
                s.push("--exec-path=");
                s.push(exec_path);
                s
            })
            .arg("maintenance")
            .arg("run")
            .arg(format!("--schedule={}", time_period.name()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(repo_path)
            .output()
            .with_context(|| {
                format!(
                    "running maintenance failed for {}",
                    repo_path.to_string_lossy()
                )
            })
    }

    fn handle_missing_config_entry<S: AsRef<str>>(&mut self, bad_entry: S) -> Result<()> {
        self.config
            .remove_multivar(&self.config_key, &regex_escape(bad_entry))?;
        Ok(())
    }

    fn get_repo_paths(&self) -> Result<Vec<String>> {
        let entries = self.config.multivar(&self.config_key, None)?;
        let vec_entries: Vec<git2::ConfigEntry> = entries
            .into_iter()
            .collect::<Result<Vec<git2::ConfigEntry>, git2::Error>>()?;

        let paths = vec_entries
            .into_iter()
            .filter_map(|v| v.value().map(|x| x.to_owned()))
            .collect();

        Ok(paths)
    }

    fn run_in_path(&self, time_period: TimePeriod, path: &Path) -> Result<()> {
        info!(?time_period, ?path, "running tasks");
        match self.run_maint(time_period, path) {
            Ok(output) => {
                let status = &output.status;
                if status.success() {
                    debug!(?time_period, ?path, "completed maintenance",)
                } else {
                    warn!(?path, exit_status = ?status, "maintenance failed");
                    {
                        let cursor = Cursor::new(output.stdout);
                        for line in cursor.lines() {
                            warn!(stdout = ?line, "stdout");
                        }
                    }
                    {
                        let cursor = Cursor::new(output.stderr);
                        for line in cursor.lines() {
                            warn!(?line, "stderr");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(?path, ?e, "failed runing git-maintenance");
            }
        }
        Ok(())
    }

    pub fn run(&mut self, time_period: TimePeriod) -> Result<()> {
        let repo_paths = self.get_repo_paths()?;

        for path in repo_paths {
            let pb: &Path = path.as_ref();
            match does_repo_exist(pb) {
                Ok(true) => self.run_in_path(time_period, pb)?,
                Ok(false) => self.handle_missing_config_entry(&path)?,
                Err(e) => {
                    error!(path = ?pb, ?e, "error in determining if path is a repo");
                }
            }
        }

        Ok(())
    }
}

// lets us test the construction of the Maintenance instance
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub git_binary_path: PathBuf,
    pub config_key: String,
    pub config_path: Option<PathBuf>,
}

pub const DEFAULT_GIT_BINARY_PATH: &str = "git";

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            git_binary_path: DEFAULT_GIT_BINARY_PATH.into(),
            config_key: DEFAULT_CONFIG_KEY.to_owned(),
            config_path: None,
        }
    }
}

impl TryFrom<RunOptions> for Runner {
    type Error = anyhow::Error;
    fn try_from(opts: RunOptions) -> Result<Self> {
        Runner::new(opts)
    }
}

trait ConfigExt {
    fn multivar_values<S: AsRef<str>>(&self, name: S, regexp: Option<S>) -> Result<Vec<String>>;

    fn is_config_key_set<S: AsRef<str>>(&mut self, key: S) -> Result<bool>;
    fn set_str_if_not_set<S: AsRef<str>>(&mut self, key: S, value: S) -> Result<()>;
}

impl ConfigExt for git2::Config {
    #[allow(dead_code)]
    fn multivar_values<S: AsRef<str>>(&self, name: S, regexp: Option<S>) -> Result<Vec<String>> {
        let configs = match regexp {
            Some(s) => self.multivar(name.as_ref(), Some(s.as_ref())),
            None => self.multivar(name.as_ref(), None),
        }?;

        let mut values: Vec<String> = Vec::new();

        for config_entry_r in configs.into_iter() {
            values.push(config_entry_r?.value().unwrap().to_owned());
        }

        Ok(values)
    }

    fn is_config_key_set<S: AsRef<str>>(&mut self, key: S) -> Result<bool> {
        match self.snapshot()?.get_bytes(key.as_ref()) {
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(e) => Err(e.into()),
            Ok(_) => Ok(true),
        }
    }

    fn set_str_if_not_set<S: AsRef<str>>(&mut self, key: S, value: S) -> Result<()> {
        if !self.is_config_key_set(&key)? {
            self.set_str(key.as_ref(), value.as_ref())?;
        }
        Ok(())
    }
}

pub(crate) fn regex_escape<S: AsRef<str>>(s: S) -> String {
    format!("^{}$", regex::escape(s.as_ref()))
}

pub fn run(cli: RunOptions, time_period: TimePeriod) -> Result<()> {
    Runner::try_from(cli)?.run(time_period)?;
    Ok(())
}

pub fn write_plist(
    opts: PlistOpts,
    _time_period: TimePeriod,
    _launch_agents_dir: &Path,
) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    launchd::write_plist(&mut buf, opts)?;
    todo!("implement writing out to LaunchAgents directory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_git_repo::ScratchGitRepo;
    use anyhow::Result;
    use tempfile::TempDir;

    struct ConfigFixture {
        pub tempdir: TempDir,
        pub config_path: PathBuf,
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

        {
            let mut config = fix.config()?;

            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/foo$", "/path/to/foo")?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/bar$", "/path/to/bar")?;

            let mut maint = Runner::new(RunOptions {
                config_path: Some(fix.config_path.to_owned()),
                ..Default::default()
            })?;

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

        {
            let mut config = fix.config()?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/foo$", "/path/to/foo")?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/bar$", "/path/to/bar")?;
        }

        {
            let maint = Runner::new(RunOptions {
                config_path: Some(fix.config_path),
                ..Default::default()
            })?;

            let paths = maint.get_repo_paths()?;
            assert_eq!(paths, vec!["/path/to/foo", "/path/to/bar"])
        }

        Ok(())
    }

    fn assert_repo_defaults_set(config: &git2::Config) {
        for (k, v) in CONFIG_DEFAULTS.iter() {
            let val = config.get_string(*k).unwrap();
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
        {
            let mut conf = fix.config()?;
            conf.set_bool("testing.testing.onetwothree", true)?;
        }

        let git_binary_path = "/path/to/bin/git";
        let config_key = "other.key";
        let config_path = fix.config_path;

        let opts = RunOptions {
            git_binary_path: git_binary_path.into(),
            config_key: config_key.into(),
            config_path: Some(config_path),
        };

        let runner = Runner::new(opts)?;

        assert_eq!(runner.git_binary_path, PathBuf::from(git_binary_path));
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
