use super::*;
use crate::git_helper::ConfigExt;
use chrono::{DateTime, Utc};
use tracing::{debug, warn};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Config {
    /// If set to false, do not run sandbox cleanup (defaults to true)
    pub cleanup_enabled: bool,
    /// Sandboxes older than this many hours will be deleted automatically.
    /// if 0 then time based cleanup is not performed and we just go by
    /// max_num_sandboxes.
    pub preserve_hours: u32,
    /// The maximum number of sandboxes we'll allow to exist on disk.
    /// this is computed after we clean up sandboxes that are older
    /// than preserve_hours
    pub max_num_sandboxes: u32,
    /// the directory to search for sandboxes. If None, then use system TMPDIR
    pub sandbox_root: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cleanup_enabled: true,
            preserve_hours: Self::DEFAULT_HOURS,
            max_num_sandboxes: Self::DEFAULT_MAX_NUM_SANDBOXES,
            sandbox_root: None,
        }
    }
}

impl Config {
    pub const DEFAULT_HOURS: u32 = 24;
    pub const DEFAULT_MAX_NUM_SANDBOXES: u32 = 500;
    const MAX_NUM_SANDBOXES_KEY: &'static str = "focus.sandbox.maxnumsandboxes";
    const PRESERVE_HOURS_KEY: &'static str = "focus.sandbox.preservehours";
    const CLEANUP_KEY: &'static str = "focus.sandbox.cleanup";

    /// Try to load the config from the global git config, falling back to defaults
    /// if the sandbox cleanup isn't configured.
    pub fn try_from_git_default() -> Result<Config> {
        let mut config = git2::Config::open_default()?;
        Self::try_from_git(&mut config)
    }

    pub fn try_from_git(config: &mut git2::Config) -> Result<Config> {
        let preserve_hours = config.get_i64_with_default(Self::PRESERVE_HOURS_KEY,  Self::DEFAULT_HOURS as i64)
            .and_then(|hours| {
                let hours =
                    if hours < 0 {
                        warn!(?hours, "invalid negative value for focus.sandbox.preservehours, using default of 24");
                        24
                    } else {
                        hours
                    };
                Ok(u32::try_from(hours)?)
            })?;

        let max_num_sandboxes: u32 = config
            .get_i64_with_default(
                Self::MAX_NUM_SANDBOXES_KEY,
                Self::DEFAULT_MAX_NUM_SANDBOXES as i64,
            )
            .and_then(|num| {
                let num_i64 = if num < 0 {
                    warn!(
                        ?num,
                        "invalid negative value for {}, using default {}",
                        Self::MAX_NUM_SANDBOXES_KEY,
                        Self::DEFAULT_MAX_NUM_SANDBOXES
                    );
                    Self::DEFAULT_MAX_NUM_SANDBOXES as i64
                } else {
                    num
                };
                Ok(u32::try_from(num_i64)?)
            })?;

        Ok(Config {
            cleanup_enabled: config.get_bool_with_default(Self::CLEANUP_KEY, true)?,
            preserve_hours,
            max_num_sandboxes,
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone)]
struct DirEnt {
    pub entry: DirEntry,
    pub mtime: DateTime<Utc>,
}

impl AsRef<Path> for DirEnt {
    fn as_ref(&self) -> &Path {
        self.entry.path()
    }
}

impl TryFrom<DirEntry> for DirEnt {
    type Error = anyhow::Error;

    fn try_from(value: DirEntry) -> Result<Self, Self::Error> {
        let md = value.metadata()?;
        let mtime = md.modified()?;
        Ok(Self {
            entry: value,
            mtime: mtime.into(),
        })
    }
}

/// Run the cleanup using the config stored in gitconfig (or defaults)
pub fn run_with_default() -> Result<()> {
    Config::try_from_git_default().and_then(|config| run(&config))
}

pub fn run(config: &Config) -> Result<()> {
    let Config {
        cleanup_enabled,
        preserve_hours,
        max_num_sandboxes,
        sandbox_root,
    } = config.clone();

    if !cleanup_enabled {
        return Ok(());
    }

    let sb_root = sandbox_root.unwrap_or_else(|| std::env::temp_dir());

    let walker = WalkDir::new(&sb_root)
        .follow_links(false)
        .max_depth(1)
        .min_depth(1)
        .same_file_system(true)
        .sort_by_file_name();

    let dirents: Vec<DirEnt> = walker
        .into_iter()
        .filter_entry(|dirent| {
            !dirent.path_is_symlink()
                && dirent.file_type().is_dir()
                && dirent
                    .file_name()
                    .to_str()
                    .map(|s| s.starts_with(NAME_PREFIX))
                    .unwrap_or(false)
        })
        .filter_map(|d| d.ok())
        .filter_map(|d| DirEnt::try_from(d).ok())
        .collect();

    let (time_expired, mut unexpired): (Vec<DirEnt>, Vec<DirEnt>) = if preserve_hours == 0 {
        (vec![], dirents)
    } else {
        let cutoff = Utc::now() - chrono::Duration::hours(preserve_hours as i64);
        dirents.into_iter().partition(|d| d.mtime < cutoff)
    };

    for dirent in time_expired.into_iter() {
        safe_delete_all(&sb_root, &dirent);
    }

    // if we still have too many sandbox directories left over after expiring the
    // ones that are older than preserve_hours, we sort by time and delete the oldest
    // N so we're below max_num_sandboxes
    if unexpired.len() > max_num_sandboxes as usize {
        unexpired.sort_unstable_by_key(|d| d.mtime);
        let upper_bound = unexpired.len() - max_num_sandboxes as usize;

        for dirent in unexpired.into_iter().take(upper_bound) {
            safe_delete_all(&sb_root, &dirent)
        }
    }

    Ok(())
}

trait IsParentOf {
    fn is_parent_of<P: AsRef<Path>>(&self, other: P) -> bool;
}

impl IsParentOf for Path {
    fn is_parent_of<P: AsRef<Path>>(&self, other: P) -> bool {
        let other = other.as_ref();
        other.ancestors().any(|p| p == self)
    }
}

fn safe_delete_all(sb_root: &Path, dirent: impl AsRef<Path>) {
    let dirent = dirent.as_ref();
    assert!(
        sb_root.is_parent_of(dirent),
        "path to delete {:?} not under root {:?}",
        dirent,
        sb_root,
    );
    debug!(?dirent, "removing expired sandbox path");
    if let Err(e) = std::fs::remove_dir_all(dirent) {
        warn!(
            ?dirent,
            ?e,
            "error cleaning up sandbox directory, continuing"
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::time::FocusTime;

    use super::*;
    use anyhow::Result;
    use filetime::FileTime;

    fn assert_cleanup_config(content: impl AsRef<str>, expect: cleanup::Config) -> Result<()> {
        use std::io::prelude::*;

        let temp = tempfile::NamedTempFile::new()?;
        writeln!(temp.as_file(), "{}", content.as_ref())?;

        {
            let mut f = temp.as_file();
            f.flush()?;
            f.sync_all()?;
        }

        let mut config = git2::Config::open(temp.path())?;
        assert_eq!(cleanup::Config::try_from_git(&mut config)?, expect);

        Ok(())
    }

    #[test]
    fn cleanup_config_defaults_when_loading_from_git() -> Result<()> {
        assert_cleanup_config("", cleanup::Config::default())
    }

    #[test]
    fn cleanup_config_try_from_git() -> Result<()> {
        assert_cleanup_config(
            r##"
[focus "sandbox"]
maxnumsandboxes = 666
preservehours = 202024
cleanup = false
"##,
            cleanup::Config {
                cleanup_enabled: false,
                preserve_hours: 202024,
                max_num_sandboxes: 666,
                ..Default::default()
            },
        )
    }

    #[test]
    fn cleanup_use_sane_defaults_for_bad_values() -> Result<()> {
        focus_testing::init_logging();
        assert_cleanup_config(
            r##"
[focus "sandbox"]
maxnumsandboxes = -1238
preservehours = potato
cleanup = 72
"##,
            cleanup::Config::default(),
        )
    }

    struct SandboxFixture {
        sb_root: TempDir,
        sandboxes: Vec<PathBuf>,
    }

    impl SandboxFixture {
        fn run(mut config: Config) -> Result<Self> {
            let sb_root = tempfile::tempdir()?;
            config.sandbox_root = Some(sb_root.path().to_owned());

            let mut paths: Vec<PathBuf> = (0..7)
                .into_iter()
                .map(|i| sb_root.path().join(format!("{}{}", NAME_PREFIX, i)))
                .collect();

            paths.reverse();

            for (i, p) in paths.iter().enumerate() {
                std::fs::create_dir(p)?;
                let ft: FocusTime = FileTime::from_last_modification_time(&p.metadata()?).into();

                let new_time: FileTime = (ft - chrono::Duration::hours(i as i64)).into();

                filetime::set_file_mtime(&p, new_time)?;
            }

            paths.sort_unstable_by_key(|p| p.metadata().unwrap().modified().unwrap());

            super::run(&config)?;

            Ok(Self {
                sb_root,
                sandboxes: paths,
            })
        }
    }

    #[test]
    fn test_time_based_cleanup() -> Result<()> {
        focus_testing::init_logging();

        let SandboxFixture {
            sb_root: _sb_root,
            sandboxes,
        } = SandboxFixture::run(Config {
            preserve_hours: 1,
            ..Default::default()
        })?;

        assert!(
            sandboxes[6].exists(),
            "sandbox {:?} was expected to exist",
            sandboxes[6]
        );

        for sb in sandboxes.iter().take(6) {
            assert!(!sb.exists(), "sandbox {:?} was expected to *not* exist", sb)
        }

        Ok(())
    }

    #[test]
    fn max_num_sandboxes_only() -> Result<()> {
        focus_testing::init_logging();

        let SandboxFixture {
            sb_root: _sb_root,
            sandboxes,
        } = SandboxFixture::run(Config {
            preserve_hours: 0,
            max_num_sandboxes: 3,
            ..Default::default()
        })?;

        // it should delete the first 4 sandboxes leaving 3
        for sb in sandboxes.iter().take(4) {
            assert!(!sb.exists(), "sandbox {:?} was expected to *not* exist", sb)
        }

        for sb in sandboxes.iter().skip(4) {
            assert!(sb.exists(), "expected path {:?} to exist", sb)
        }

        Ok(())
    }

    #[test]
    fn disabled_does_nothing() -> Result<()> {
        focus_testing::init_logging();

        let SandboxFixture {
            sb_root: _sb_root,
            sandboxes,
        } = SandboxFixture::run(Config {
            cleanup_enabled: false,
            ..Default::default()
        })?;

        for sb in sandboxes.iter() {
            assert!(sb.exists(), "expected path {:?} to exist", sb)
        }

        Ok(())
    }

    #[test]
    #[should_panic]
    fn safe_delete_all_panics_if_path_is_not_under_sb_root() {
        focus_testing::init_logging();
        let root = PathBuf::from("/foo/bar/baz");
        let other = PathBuf::from("/foo/what/bar/baz");

        safe_delete_all(&root, other)
    }
}
