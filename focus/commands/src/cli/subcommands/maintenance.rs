use std::{
    ffi::OsString,
    io::{BufRead, Cursor},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result};
use focus_internals::util::git_helper;

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub enum TimePeriod {
    Hourly,
    Daily,
    Weekly,
}

impl TimePeriod {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }
}

const DEFAULT_CONFIG_KEY: &str = "maintenance.repo";

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

fn is_config_key_set<S: AsRef<str>>(config: &mut git2::Config, key: S) -> Result<bool> {
    match config.snapshot()?.get_bytes(key.as_ref()) {
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(true),
    }
}

fn set_str_if_not_set<S: AsRef<str>>(config: &mut git2::Config, key: S, value: S) -> Result<()> {
    if !is_config_key_set(config, &key)? {
        config.set_str(key.as_ref(), value.as_ref())?;
    }
    Ok(())
}

/// Configures the repo at `path` to have git-maintenance run the standard jobs
#[allow(dead_code)]
pub fn set_default_repo_config(config: &mut git2::Config) -> Result<()> {
    for (k, v) in CONFIG_DEFAULTS.iter() {
        set_str_if_not_set(config, *k, *v)?
    }

    Ok(())
}

pub struct Maintenance {
    pub git_binary_path: PathBuf,
    pub exec_path: PathBuf,
    /// the config key in the global git config that contains the list of paths to check.
    /// By default this is "maintenance.repo", a multi value key.
    pub config_key: String,
    pub config: git2::Config,
}

#[allow(dead_code)]
impl Maintenance {
    pub fn new(config_opt: Option<git2::Config>) -> Result<Maintenance> {
        let config = match config_opt {
            Some(c) => c,
            None => git2::Config::open_default()?,
        }
        .open_level(git2::ConfigLevel::Global)?;

        let git_binary_path = git_helper::git_binary_path()?;
        let exec_path = git_helper::git_exec_path()?;
        let config_key = DEFAULT_CONFIG_KEY.to_owned();

        Ok(Maintenance {
            git_binary_path,
            exec_path,
            config_key,
            config,
        })
    }

    fn does_repo_exist(path: &Path) -> Result<bool> {
        match git2::Repository::discover(path) {
            Err(git_err) if git_err.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(e) => Err(e.into()),
            Ok(_) => Ok(true),
        }
    }

    fn run_maint(&self, time_period: TimePeriod, repo_path: &Path) -> Result<Output> {
        // TODO: this needs to log and capture output for debugging if necessary
        Command::new(&self.git_binary_path)
            .arg({
                let mut s = OsString::new();
                s.push("--exec-path=");
                s.push(&self.exec_path);
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
        self.config.remove_multivar(
            &self.config_key,
            &format!("^{}$", regex::escape(bad_entry.as_ref())),
        )?;
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
        match self.run_maint(time_period, path) {
            Ok(output) => {
                let status = &output.status;
                if status.success() {
                    log::debug!(
                        "completed {} maintenance for {:?}",
                        time_period.name(),
                        path
                    )
                } else {
                    log::warn!("maintenance failed for {:?}, exit status {}", path, status);
                    {
                        let cursor = Cursor::new(output.stdout);
                        for line in cursor.lines() {
                            log::warn!("stdout: {}", line?)
                        }
                    }
                    {
                        let cursor = Cursor::new(output.stderr);
                        for line in cursor.lines() {
                            log::warn!("stderr: {}", line?)
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("failed runing git-maintenance for repo: {:?}", path);
                log::warn!("{:?}", e);
            }
        }
        Ok(())
    }

    pub fn run(&mut self, time_period: TimePeriod) -> Result<()> {
        let repo_paths = self.get_repo_paths()?;

        for path in repo_paths {
            let pb: &Path = path.as_ref();
            match Self::does_repo_exist(pb) {
                Ok(true) => self.run_in_path(time_period, pb)?,
                Ok(false) => self.handle_missing_config_entry(&path)?,
                Err(e) => {
                    log::error!("error in determining if path {:?} is a repo", pb);
                    log::error!("{:?}", e);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ops::{Deref, DerefMut},
        str,
    };

    use super::*;
    use anyhow::Result;
    use focus_internals::testing::scratch_git_repo::ScratchGitRepo;
    use tempfile::TempDir;

    struct ConfigExt(git2::Config);

    impl Deref for ConfigExt {
        type Target = git2::Config;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl DerefMut for ConfigExt {
        fn deref_mut(&mut self) -> &mut git2::Config {
            &mut self.0
        }
    }

    impl AsRef<git2::Config> for ConfigExt {
        fn as_ref(&self) -> &git2::Config {
            &self.0
        }
    }

    impl ConfigExt {
        pub fn new(config: git2::Config) -> Self {
            Self(config)
        }

        pub fn multivar_values<S: AsRef<str>>(
            &self,
            name: S,
            regexp: Option<S>,
        ) -> Result<Vec<String>> {
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

        pub fn into_inner(self) -> git2::Config {
            self.0
        }
    }

    struct ConfigFixture {
        _tempdir: TempDir,
        config_path: PathBuf,
    }

    impl ConfigFixture {
        fn new() -> Result<ConfigFixture> {
            let tempdir = TempDir::new()?;
            let path = tempdir.path().join("global.config");
            {
                std::fs::File::create(&path)?;
            }
            Ok(ConfigFixture {
                _tempdir: tempdir,
                config_path: path,
            })
        }

        fn config(&self) -> Result<ConfigExt> {
            let mut config = git2::Config::new()?;
            config.add_file(&self.config_path, git2::ConfigLevel::Global, true)?;
            Ok(ConfigExt::new(config))
        }
    }

    #[test]
    fn test_handle_missing_config_entry() -> Result<()> {
        let fix = ConfigFixture::new()?;

        {
            let mut config = fix.config()?;

            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/foo$", "/path/to/foo")?;
            config.set_multivar(DEFAULT_CONFIG_KEY, "^/path/to/bar$", "/path/to/bar")?;

            let mut maint = Maintenance::new(Some(config.into_inner()))?;

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
            let config = fix.config()?;
            let maint = Maintenance::new(Some(config.into_inner()))?;

            let paths = maint.get_repo_paths()?;
            assert_eq!(paths, vec!["/path/to/foo", "/path/to/bar"])
        }

        Ok(())
    }

    #[test]
    fn test_set_defalts() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_fixture(temp.path())?;
        let repo = fix.repo()?;
        let mut config = repo.config()?;

        set_default_repo_config(&mut config)?;
        for (k, v) in CONFIG_DEFAULTS.iter() {
            let val = config.get_string(*k)?;
            assert_eq!(
                val, *v,
                "values for key {} were not equal: {} != {}",
                *k, val, *v
            );
        }

        Ok(())
    }
}
