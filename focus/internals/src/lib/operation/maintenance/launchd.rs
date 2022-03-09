use super::*;
use anyhow::{bail, Result};
use plist::{dictionary::Dictionary, Value};
use std::{io::Write, path::PathBuf};
use strum::IntoEnumIterator;
use tracing::{debug, error};

pub const DEFAULT_FOCUS_PATH: &str = "/opt/twitter_mde/bin/focus";
pub const DEFAULT_GIT_BINARY_PATH_FOR_PLISTS: &str = "/opt/twitter_mde/bin/git";

#[derive(Debug, Clone)]
pub struct PlistOpts {
    pub focus_path: PathBuf,
    pub git_binary_path: PathBuf,
    pub time_period: TimePeriod,
    pub config_key: String,
    pub config_path: Option<String>,
    pub schedule_defaults: Option<CalendarInterval>,
}

impl PlistOpts {
    pub fn label(&self) -> String {
        format!("com.twitter.git-maintenance.{}", self.time_period)
    }
}

impl Default for PlistOpts {
    fn default() -> Self {
        Self {
            focus_path: DEFAULT_FOCUS_PATH.into(),
            git_binary_path: DEFAULT_GIT_BINARY_PATH_FOR_PLISTS.into(),
            time_period: TimePeriod::Hourly,
            config_key: DEFAULT_CONFIG_KEY.into(),
            config_path: None,
            schedule_defaults: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProgramArguments(PlistOpts);

impl From<PlistOpts> for ProgramArguments {
    fn from(t: PlistOpts) -> Self {
        Self(t)
    }
}

impl From<ProgramArguments> for Value {
    fn from(args: ProgramArguments) -> Self {
        let ProgramArguments(PlistOpts {
            focus_path,
            git_binary_path,
            time_period,
            config_key,
            config_path,
            schedule_defaults: _,
        }) = args;

        let config_key = format!("--config-key={}", config_key);
        let git_binary_path = format!("--git-binary-path={}", git_binary_path.to_str().unwrap());
        let time_period = format!("--time-period={}", time_period);

        let mut args: Vec<String> = vec![
            focus_path.to_str().unwrap().to_owned(),
            "maintenance".into(),
            config_key,
            "run".into(),
            git_binary_path,
            time_period,
        ];

        if let Some(config_path) = config_path.as_deref() {
            args.push(format!("--config-path={}", config_path));
        }

        Value::Array(args.into_iter().map(|a| a.into()).collect())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CalendarInterval {
    day: Option<u32>,
    hour: Option<u32>,
    minute: Option<u32>,
    weekday: Option<u32>,
}

#[allow(dead_code)]
const DEFAULT_DAILY_HOUR: u32 = 4;
const DEFAULT_WEEKLY_WEEKDAY: u32 = 1;

fn random_minute() -> u32 {
    rand::random::<u32>() % 60
}

impl CalendarInterval {
    fn daily(minute: u32, hour: u32) -> Vec<CalendarInterval> {
        (0..7)
            .into_iter()
            .map(|weekday| CalendarInterval {
                weekday: Some(weekday),
                hour: Some(hour),
                minute: Some(minute),
                ..Default::default()
            })
            .collect()
    }

    fn hourly(minute: u32) -> Vec<CalendarInterval> {
        (0..24)
            .into_iter()
            .map(|hour: u32| CalendarInterval {
                hour: Some(hour),
                minute: Some(minute),
                ..Default::default()
            })
            .collect()
    }

    fn weekly(minute: u32, hour: u32, weekday: u32) -> Vec<CalendarInterval> {
        vec![CalendarInterval {
            weekday: Some(weekday),
            hour: Some(hour),
            minute: Some(minute),
            ..Default::default()
        }]
    }

    pub(crate) fn for_time_period(
        tp: TimePeriod,
        defaults: CalendarInterval,
    ) -> Vec<CalendarInterval> {
        match tp {
            TimePeriod::Hourly => Self::hourly(defaults.minute.unwrap_or_else(random_minute)),
            TimePeriod::Daily => {
                let CalendarInterval {
                    day: _,
                    hour,
                    minute,
                    weekday: _,
                } = defaults;
                Self::daily(
                    minute.unwrap_or_else(random_minute),
                    hour.unwrap_or(DEFAULT_DAILY_HOUR),
                )
            }
            TimePeriod::Weekly => {
                let CalendarInterval {
                    day: _,
                    hour,
                    minute,
                    weekday,
                } = defaults;
                Self::weekly(
                    minute.unwrap_or_else(random_minute),
                    hour.unwrap_or(DEFAULT_DAILY_HOUR),
                    weekday.unwrap_or(DEFAULT_WEEKLY_WEEKDAY),
                )
            }
        }
    }
}

impl From<CalendarInterval> for Value {
    fn from(ci: CalendarInterval) -> Self {
        (&ci).into()
    }
}

impl From<&CalendarInterval> for Value {
    fn from(t: &CalendarInterval) -> Value {
        let CalendarInterval {
            day,
            hour,
            minute,
            weekday,
        } = t;

        let mut dict = Dictionary::new();

        if let Some(day) = day {
            dict.insert("Day".into(), day.into());
        }

        if let Some(hour) = hour {
            dict.insert("Hour".into(), hour.into());
        }

        if let Some(minute) = minute {
            dict.insert("Minute".into(), minute.into());
        }

        if let Some(weekday) = weekday {
            dict.insert("Weekday".into(), weekday.into());
        }

        dict.into()
    }
}

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

impl From<LaunchdPlist> for Value {
    fn from(t: LaunchdPlist) -> Self {
        let mut dict = Dictionary::new();
        dict.insert(DISABLED.into(), t.disabled.into());
        dict.insert(LABEL.into(), t.label.deref().into());
        dict.insert(PROGRAM_ARGUMENTS.into(), t.program_args.clone().into());
        dict.insert(
            START_CALENDAR_INTERVAL.into(),
            Value::Array(
                t.start_calendar_interval
                    .iter()
                    .map(|ci| ci.into())
                    .collect(),
            ),
        );
        dict.into()
    }
}

impl From<PlistOpts> for LaunchdPlist {
    fn from(plist_opts: PlistOpts) -> Self {
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

pub fn write_plist<W: Write>(writer: W, plist_opts: PlistOpts) -> Result<()> {
    let lp: LaunchdPlist = plist_opts.into();

    let v: Value = lp.into();

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
            _ =>  panic!("args must have at least two items"),
        };
        assert!(args.len() >= 2, "args must have at least 2 items");

        debug!("running launchctl {:?}", args);

        let res = Command::new(&self.launchctl_bin)
            .arg(cmd)
            .arg(target)
            .args(rest)
            .spawn()?
            .wait()?;

        if !res.success() {
            let msg = format!("failed to run launchctl {} {}: {}", cmd, target, res);
            error!("launchctl error: {}", msg);
            bail!(msg)
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

    pub fn write_plist(&self, opts: &PlistOpts) -> Result<PathBuf> {
        let output_path = self.plist_path(&opts.label());

        let mut temp = tempfile::NamedTempFile::new_in(&self.launch_agents_path)?;
        launchd::write_plist(&mut temp, opts.clone())?;
        temp.as_file().sync_all()?;
        std::fs::rename(temp.path(), &output_path)?;

        Ok(output_path)
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
    pub skip_if_running: bool,
}

impl Default for ScheduleOpts {
    fn default() -> Self {
        Self {
            time_period: Default::default(),
            git_path: Default::default(),
            focus_path: Default::default(),
            skip_if_running: true,
        }
    }
}

/// This is the function that main calls to write out the plists and load them.
/// If time_period is None that means "all"
#[tracing::instrument]
pub fn schedule_enable(opts: ScheduleOpts) -> Result<()> {
    let ScheduleOpts {
        time_period,
        git_path,
        focus_path,
        skip_if_running,
    } = opts;

    let launchctl = Launchctl::default();

    let time_periods: Vec<TimePeriod> = match time_period {
        Some(tp) => vec![tp],
        None => TimePeriod::iter().collect(),
    };

    let plist_opts = PlistOpts {
        focus_path,
        git_binary_path: git_path,
        ..Default::default()
    };

    for tp in time_periods {
        let plist_opts = PlistOpts {
            time_period: tp,
            ..plist_opts.clone()
        };
        let label = plist_opts.label();

        launchctl.write_plist(&plist_opts)?;

        if launchctl.is_service_loaded(&label)? {
            // the service is already registered and scheduled
            if skip_if_running {
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
pub fn schedule_disable() -> Result<()> {
    let launchctl = Launchctl::default();
    let time_periods: Vec<TimePeriod> = TimePeriod::iter().collect();

    for tp in time_periods {
        let plist_opts = PlistOpts {
            time_period: tp,
            ..Default::default()
        };

        let label = plist_opts.label();

        if launchctl.is_service_loaded(&label)? {
            launchctl.bootout(&label)?; // otherwise stop the service and unload it
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use plist;

    fn plist_opts_fix() -> PlistOpts {
        PlistOpts {
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
        }
    }

    #[test]
    fn test_serialize_launchdplist_value() -> Result<()> {
        let plist_opts = plist_opts_fix();

        let v: plist::Value = LaunchdPlist::from(plist_opts).into();

        insta::assert_yaml_snapshot!(v);

        Ok(())
    }
}
