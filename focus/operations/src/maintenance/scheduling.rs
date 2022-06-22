// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

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

// TODO: this is both a serializable object for outputting a .plist and also
// used as an input, hence the `every_n_minutes` field that is *not* convertible
// into a field in a plist. Separate these two concepts so they don't overlap
// (i.e. input/options struct and data model object)
#[derive(Debug, Clone, Default)]
pub struct CalendarInterval {
    /// Day of month (1-31)
    pub day: Option<u32>,
    /// Hour of day (0-23)
    pub hour: Option<u32>,
    /// Minute of hour (0-59)
    pub minute: Option<u32>,
    /// Weekday 0 and 7 are both Sunday
    pub weekday: Option<u32>,
    /// When used with Hourly period, will create intervals every
    /// N minutes, with an offset given by minute. This logic
    /// isn't terribly sophisticated, so don't try to do
    /// anything too fancy with it.
    pub every_n_minutes: Option<u32>,
}

#[allow(dead_code)]
const DEFAULT_DAILY_HOUR: u32 = 4;
const DEFAULT_WEEKLY_WEEKDAY: u32 = 1;

fn random_minute() -> u32 {
    rand::random::<u32>() % 60
}

fn random_offset() -> u32 {
    rand::random::<u32>() % 10
}

impl CalendarInterval {
    fn every_n_minute_interval(n: u32, offset: u32) -> Vec<u32> {
        assert!(n > 0, "{n} was expected to be > 0");
        assert!(offset < 10, "offset {offset} must be <10");

        (0..60)
            .into_iter()
            .step_by(n.try_into().unwrap())
            .map(|i| i + offset)
            .take_while(|i| *i < 60)
            .collect()
    }

    fn every_n_minutes(n: u32, offset: u32) -> Vec<CalendarInterval> {
        Self::every_n_minute_interval(n, offset)
            .into_iter()
            .map(|min| CalendarInterval {
                minute: Some(min),
                ..Default::default()
            })
            .collect()
    }

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
            TimePeriod::Hourly => match defaults {
                CalendarInterval {
                    day: _,
                    hour: _,
                    minute: Some(offset),
                    weekday: _,
                    every_n_minutes: Some(n),
                } => Self::every_n_minutes(n, offset),

                CalendarInterval {
                    day: _,
                    hour: _,
                    minute: None,
                    weekday: _,
                    every_n_minutes: Some(n),
                } => Self::every_n_minutes(n, random_offset()),

                CalendarInterval {
                    day: _,
                    hour: _,
                    minute: Some(min),
                    weekday: _,
                    every_n_minutes: None,
                } => Self::hourly(min),

                CalendarInterval {
                    day: _,
                    hour: _,
                    minute: None,
                    weekday: _,
                    every_n_minutes: None,
                } => Self::hourly(random_minute()),
            },
            TimePeriod::Daily => {
                let CalendarInterval {
                    day: _,
                    hour,
                    minute,
                    weekday: _,
                    every_n_minutes: _,
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
                    every_n_minutes: _,
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
            every_n_minutes: _,
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
