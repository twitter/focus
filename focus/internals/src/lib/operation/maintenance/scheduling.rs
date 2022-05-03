use std::{fmt::Debug, path::PathBuf};

use super::*;

pub(crate) type PlistValue = plist::Value;
pub(crate) type PlistDictionary = plist::Dictionary;

#[derive(Debug, Clone)]
pub struct ScheduledJobOpts {
    pub focus_path: PathBuf,
    pub git_binary_path: PathBuf,
    pub time_period: TimePeriod,
    pub config_key: String,
    pub config_path: Option<String>,
    pub schedule_defaults: Option<CalendarInterval>,
    /// run maintenance on all tracked repos
    pub tracked: bool,
}

impl ScheduledJobOpts {
    pub fn label(&self) -> String {
        format!("com.twitter.git-maintenance.{}", self.time_period)
    }
}

impl Default for ScheduledJobOpts {
    fn default() -> Self {
        Self {
            focus_path: DEFAULT_FOCUS_PATH.into(),
            git_binary_path: DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS.into(),
            time_period: TimePeriod::Hourly,
            config_key: DEFAULT_CONFIG_KEY.into(),
            config_path: None,
            schedule_defaults: None,
            tracked: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProgramArguments(ScheduledJobOpts);

impl From<ScheduledJobOpts> for ProgramArguments {
    fn from(t: ScheduledJobOpts) -> Self {
        Self(t)
    }
}

impl From<ProgramArguments> for PlistValue {
    fn from(args: ProgramArguments) -> Self {
        let ProgramArguments(ScheduledJobOpts {
            focus_path,
            git_binary_path,
            time_period,
            config_key,
            config_path,
            tracked,
            schedule_defaults: _,
        }) = args;

        assert!(
            !git_binary_path.as_os_str().is_empty(),
            "git_binary_path is empty"
        );

        let config_key = format!("--git-config-key={}", config_key);
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

        if tracked {
            args.push("--tracked".into());
        }

        PlistValue::Array(args.into_iter().map(|a| a.into()).collect())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CalendarInterval {
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub weekday: Option<u32>,
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

impl From<CalendarInterval> for PlistValue {
    fn from(ci: CalendarInterval) -> Self {
        (&ci).into()
    }
}

impl From<&CalendarInterval> for PlistValue {
    fn from(t: &CalendarInterval) -> PlistValue {
        let CalendarInterval {
            day,
            hour,
            minute,
            weekday,
        } = t;

        let mut dict = PlistDictionary::new();

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
