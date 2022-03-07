use super::*;
use plist::{dictionary::Dictionary, Value};
use std::{io::Write, path::PathBuf};

#[derive(Debug, Clone)]
pub struct PlistOpts {
    pub focus_path: PathBuf,
    pub git_binary_path: PathBuf,
    pub time_period: TimePeriod,
    pub config_key: String,
    pub config_path: Option<String>,
    pub schedule_defaults: Option<CalendarInterval>,
}

impl Default for PlistOpts {
    fn default() -> Self {
        Self {
            focus_path: PathBuf::new(),
            git_binary_path: PathBuf::new(),
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
        (0..6)
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
        (0..23)
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
            label: format!("com.twitter.git-maintenance.{}", plist_opts.time_period),
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

    plist::to_writer_xml(writer, &v)?;
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
