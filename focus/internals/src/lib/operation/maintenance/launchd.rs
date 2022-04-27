use super::{scheduling::*, *};
use anyhow::{anyhow, bail, Context, Result};
use std::{
    io::{ErrorKind, Write},
    path::PathBuf,
};
use strum::IntoEnumIterator;
use tracing::{debug, error};

#[derive(Debug, Clone, Default)]
struct LaunchdPlist {
    pub disabled: bool,
    pub label: String,
    pub program_args: ProgramArguments,
    pub start_calendar_interval: Vec<CalendarInterval>,
}

const DISABLED: &str = "Disabled";
const LABEL: &str = "Label";
const PROGRAM_ARGUMENTS: &str = "ProgramArguments";
const START_CALENDAR_INTERVAL: &str = "StartCalendarInterval";

impl From<LaunchdPlist> for PlistValue {
    fn from(t: LaunchdPlist) -> Self {
        let mut dict = PlistDictionary::new();
        dict.insert(DISABLED.into(), t.disabled.into());
        dict.insert(LABEL.into(), t.label.deref().into());
        dict.insert(PROGRAM_ARGUMENTS.into(), t.program_args.clone().into());
        dict.insert(
            START_CALENDAR_INTERVAL.into(),
            PlistValue::Array(
                t.start_calendar_interval
                    .iter()
                    .map(|ci| ci.into())
                    .collect(),
            ),
        );
        dict.into()
    }
}

impl From<ScheduledJobOpts> for LaunchdPlist {
    fn from(plist_opts: ScheduledJobOpts) -> Self {
        LaunchdPlist {
            disabled: false,
            label: plist_opts.label(),
            program_args: plist_opts.clone().into(),
            start_calendar_interval: CalendarInterval::for_time_period(
                plist_opts.time_period,
                plist_opts.schedule_defaults.unwrap_or_default(),
            ),
        }
    }
}

pub fn write_plist<W: Write>(writer: W, plist_opts: ScheduledJobOpts) -> Result<()> {
    let lp: LaunchdPlist = plist_opts.into();

    let v: PlistValue = lp.into();

    Ok(plist::to_writer_xml(writer, &v)?)
}

const LAUNCHCTL_BIN: &str = "/bin/launchctl";
const LAUNCH_AGENTS_RELPATH: &str = "Library/LaunchAgents";

trait CommandExt {
    fn debug_command_line(&self) -> OsString;
}

impl CommandExt for Command {
    fn debug_command_line(&self) -> OsString {
        let mut s = self.get_program().to_owned();
        for arg in self.get_args() {
            s.push(" ");
            s.push(arg)
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct Launchctl {
    pub launchctl_bin: PathBuf,
    pub launch_agents_path: PathBuf,
}

impl Launchctl {
    fn gui_domain_id() -> String {
        let uid = nix::unistd::Uid::current();
        format!("gui/{}", uid)
    }

    fn gui_service_id<S: AsRef<str>>(label: S) -> String {
        format!("{}/{}", Self::gui_domain_id(), label.as_ref())
    }

    #[tracing::instrument]
    fn is_service_loaded_os_str(&self, label: &str) -> Result<bool> {
        let svc_id = Self::gui_service_id(label);
        let out = Command::new(&self.launchctl_bin)
            .arg("print")
            .arg(svc_id)
            .output()?;

        debug!(label = ?label, success = ?out.status.success(), "is service loaded");
        Ok(out.status.success())
    }

    /// Returns true if the given service is loaded into launchd.
    /// The process may be *stopped* but launchd knows of its existence.
    pub fn is_service_loaded<S: AsRef<str>>(&self, label: S) -> Result<bool> {
        self.is_service_loaded_os_str(label.as_ref())
    }

    #[tracing::instrument]
    fn exec_cmd(&self, args: &[&str]) -> Result<()> {
        let (cmd, target, rest) = match args {
            [cmd, target, args @ ..] => (*cmd, *target, args),
            _ => panic!("args must have at least two items"),
        };
        assert!(args.len() >= 2, "args must have at least 2 items");

        debug!("running launchctl {:?}", args);

        let fail_msg = format!("failed to run launchctl {} {}", cmd, target);

        let res = Command::new(&self.launchctl_bin)
            .arg(cmd)
            .arg(target)
            .args(rest)
            .spawn()
            .context(fail_msg.to_owned())?
            .wait()?;

        if !res.success() {
            let fail_msg = format!("launchctl error: {}: result {}", fail_msg, res);
            error!("{}", fail_msg);
            bail!(fail_msg)
        }

        Ok(())
    }

    pub fn enable<S: AsRef<str>>(&self, label: S) -> Result<()> {
        let label = label.as_ref();
        self.exec_cmd(&["enable", Self::gui_service_id(label).as_str()])
    }

    pub fn bootstrap<S: AsRef<str>>(&self, label: S) -> Result<()> {
        self.exec_cmd(&[
            "bootstrap",
            &Self::gui_domain_id(),
            self.plist_path(label.as_ref())
                .to_str()
                .expect("plist path was not valid UTF-8"),
        ])
    }

    pub fn bootout<S: AsRef<str>>(&self, label: S) -> Result<()> {
        self.exec_cmd(&["bootout", Self::gui_service_id(label).as_str()])
    }

    pub fn plist_path(&self, label: &str) -> PathBuf {
        let mut output_path = self.launch_agents_path.to_path_buf();
        output_path.push(format!("{}.plist", label));
        output_path
    }

    pub fn write_plist(&self, opts: &ScheduledJobOpts) -> Result<PathBuf> {
        let output_path = self.plist_path(&opts.label());

        let mut temp = tempfile::NamedTempFile::new_in(&self.launch_agents_path)?;
        launchd::write_plist(&mut temp, opts.clone())?;
        temp.as_file().sync_all()?;
        std::fs::rename(temp.path(), &output_path)?;

        Ok(output_path)
    }

    pub fn delete_plist(&self, opts: &ScheduledJobOpts) -> Result<()> {
        let path = self.plist_path(&opts.label());
        let res = std::fs::remove_file(&path);

        let checked = match res {
            Err(e) => match e.kind() {
                ErrorKind::NotFound => Ok(()),
                _ => Err(anyhow!(e)),
            },
            _ => Ok(()),
        };

        checked.with_context(|| format!("failed to remove path {:?}", path))
    }
}

impl Default for Launchctl {
    fn default() -> Self {
        let home = dirs::home_dir().expect("could not determine HOME dir");

        Self {
            launchctl_bin: LAUNCHCTL_BIN.into(),
            launch_agents_path: home.join(LAUNCH_AGENTS_RELPATH),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduleOpts {
    pub time_period: Option<TimePeriod>,
    pub git_path: PathBuf,
    pub focus_path: PathBuf,
    pub skip_if_already_scheduled: bool,
    pub tracked: bool,
}

impl Default for ScheduleOpts {
    fn default() -> Self {
        Self {
            time_period: Default::default(),
            git_path: DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS.into(),
            focus_path: std::env::current_exe()
                .expect("could not determine current executable path"),
            skip_if_already_scheduled: true,
            tracked: true,
        }
    }
}

#[tracing::instrument]
#[cfg(target_os = "linux")]
pub fn schedule_enable(opts: ScheduleOpts) -> Result<()> {
    Ok(())
}

/// This is the function that main calls to write out the plists and load them.
/// If time_period is None that means "all"
#[tracing::instrument]
#[cfg(target_os = "macos")]
pub fn schedule_enable(opts: ScheduleOpts) -> Result<()> {
    let ScheduleOpts {
        time_period,
        git_path,
        focus_path,
        skip_if_already_scheduled,
        tracked,
    } = opts;

    assert!(
        git_path.is_absolute(),
        "git_path must be absolute: {:?}",
        git_path
    );
    assert!(
        focus_path.is_absolute(),
        "focus_path must be absolute: {:?}",
        focus_path
    );

    let launchctl = Launchctl::default();

    let time_periods: Vec<TimePeriod> = match time_period {
        Some(tp) => vec![tp],
        None => TimePeriod::iter().collect(),
    };

    let plist_opts = ScheduledJobOpts {
        focus_path,
        git_binary_path: git_path,
        tracked,
        ..Default::default()
    };

    for tp in time_periods {
        let plist_opts = ScheduledJobOpts {
            time_period: tp,
            ..plist_opts.clone()
        };
        let label = plist_opts.label();

        launchctl.write_plist(&plist_opts)?;

        if launchctl.is_service_loaded(&label)? {
            // the service is already registered and scheduled
            if skip_if_already_scheduled {
                continue; // and a forced reload hasn't been requested, so check the next one
            } else {
                launchctl.bootout(&label)?; // otherwise stop the service and unload it
            }
        }

        launchctl.enable(&label)?;
        launchctl.bootstrap(&label)?;
    }

    Ok(())
}

#[tracing::instrument]
pub fn schedule_disable(delete: bool) -> Result<()> {
    let launchctl = Launchctl::default();
    let time_periods: Vec<TimePeriod> = TimePeriod::iter().collect();

    for tp in time_periods {
        let plist_opts = ScheduledJobOpts {
            time_period: tp,
            git_binary_path: DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS.into(),
            ..Default::default()
        };

        let label = plist_opts.label();

        if launchctl.is_service_loaded(&label)? {
            launchctl.bootout(&label)?; // otherwise stop the service and unload it
        }

        if delete {
            launchctl.delete_plist(&plist_opts)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn plist_opts_fix() -> ScheduledJobOpts {
        ScheduledJobOpts {
            focus_path: "/path/to/focus".into(),
            git_binary_path: "/usr/local/bin/git".into(),
            time_period: TimePeriod::Hourly,
            config_key: DEFAULT_CONFIG_KEY.into(),
            config_path: Some("/path/to/config".to_string()),
            schedule_defaults: Some(CalendarInterval {
                hour: Some(4),
                minute: Some(12),
                ..Default::default()
            }),
            tracked: true,
        }
    }

    #[test]
    fn test_serialize_plist_value() -> Result<()> {
        let plist_opts = plist_opts_fix();

        let v: PlistValue = LaunchdPlist::from(plist_opts).into();

        insta::assert_yaml_snapshot!(v);

        Ok(())
    }
}
