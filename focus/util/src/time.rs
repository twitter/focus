// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use filetime::FileTime;
use std::borrow::Borrow;
use std::fmt;
use std::ops::{Add, Deref, Sub};

use chrono::{
    offset, Date, DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone,
    Timelike, Utc,
};
use git2::Time;

static DATE_FORMAT: &str = "%Y-%m-%d";

pub trait ToRFC3339 {
    fn to_rfc3339(&self) -> String;
}

pub fn local_timestamp_rfc3339() -> String {
    Local::now().to_rfc3339()
}

pub fn date_at_day_in_past(days_into_past: i64) -> Result<Date<Utc>> {
    let today = Utc::now().date();
    today
        .checked_sub_signed(Duration::days(days_into_past))
        .with_context(|| format!("Could not determine date {} days ago", days_into_past))
}

pub fn formatted_datestamp_at_day_in_past(days_into_past: i64) -> Result<String> {
    let datestamp = date_at_day_in_past(days_into_past)?;
    let formatted_datestamp = datestamp.format(DATE_FORMAT);
    Ok(formatted_datestamp.to_string())
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitTime(Time);

impl GitTime {
    pub fn into_inner(self) -> Time {
        self.0
    }

    pub fn new(t: Time) -> GitTime {
        GitTime(t)
    }
}

impl AsRef<Time> for GitTime {
    fn as_ref(&self) -> &Time {
        &self.0
    }
}

impl Deref for GitTime {
    type Target = Time;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for GitTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitTime")
            .field("seconds", &self.0.seconds())
            .field("offset_minutes", &self.0.offset_minutes())
            .finish()
    }
}

impl From<Time> for GitTime {
    fn from(t: Time) -> Self {
        GitTime(t)
    }
}

impl From<DateTime<FixedOffset>> for GitTime {
    fn from(dt: DateTime<FixedOffset>) -> Self {
        GitTime(Time::new(
            dt.timestamp(),
            dt.offset().local_minus_utc() / 60,
        ))
    }
}

impl From<FocusTime> for GitTime {
    fn from(t: FocusTime) -> Self {
        GitTime::from(t.0)
    }
}

impl From<&FocusTime> for GitTime {
    fn from(t: &FocusTime) -> Self {
        GitTime::from(t.0)
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct FocusTime(DateTime<FixedOffset>);

impl FocusTime {
    pub fn into_inner(self) -> DateTime<FixedOffset> {
        self.0
    }

    pub fn parse_date<S: AsRef<str>>(s: S) -> Result<FocusTime> {
        Ok(FocusTime(
            NaiveDate::parse_from_str(s.as_ref(), DATE_FORMAT)
                .map(|nd| Date::from_utc(nd, offset::FixedOffset::west(0)).and_hms(0, 0, 0))?,
        ))
    }

    pub fn parse_from_rfc3339<S: AsRef<str>>(s: S) -> Result<FocusTime> {
        Ok(FocusTime(DateTime::parse_from_rfc3339(s.as_ref())?))
    }

    pub fn now() -> FocusTime {
        let localnow = Local::now();
        let fixed = FixedOffset::from_offset(localnow.offset()).from_utc_datetime(
            &NaiveDateTime::from_timestamp(localnow.timestamp(), localnow.nanosecond()),
        );
        FocusTime(fixed)
    }
}

impl From<FileTime> for FocusTime {
    fn from(ft: FileTime) -> Self {
        Self(FixedOffset::east(0).timestamp(ft.seconds(), ft.nanoseconds()))
    }
}

impl From<FocusTime> for FileTime {
    fn from(ft: FocusTime) -> Self {
        FileTime::from_unix_time(ft.timestamp(), ft.nanosecond())
    }
}

impl Sub<Duration> for FocusTime {
    type Output = FocusTime;

    fn sub(self, d: Duration) -> Self::Output {
        FocusTime(self.0 - d)
    }
}

impl Add<Duration> for FocusTime {
    type Output = FocusTime;

    fn add(self, d: Duration) -> Self::Output {
        FocusTime(self.0 + d)
    }
}

impl Borrow<DateTime<FixedOffset>> for FocusTime {
    fn borrow(&self) -> &DateTime<FixedOffset> {
        &self.0
    }
}

impl Deref for FocusTime {
    type Target = DateTime<FixedOffset>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<git2::Time> for FocusTime {
    fn from(t: git2::Time) -> Self {
        FocusTime(DateTime::from_utc(
            NaiveDateTime::from_timestamp(t.seconds(), 0 /* nanos */),
            FixedOffset::west(t.offset_minutes() * 60),
        ))
    }
}

impl From<GitTime> for FocusTime {
    fn from(gt: GitTime) -> Self {
        Self::from(gt.into_inner())
    }
}

pub struct GitIdentTime(FocusTime);

impl GitIdentTime {
    pub fn parse_from_rfc3339<S: AsRef<str>>(s: S) -> Result<GitIdentTime> {
        FocusTime::parse_from_rfc3339(s).map(GitIdentTime)
    }
}

impl From<FocusTime> for GitIdentTime {
    fn from(t: FocusTime) -> Self {
        GitIdentTime(t)
    }
}

impl From<&FocusTime> for GitIdentTime {
    fn from(t: &FocusTime) -> Self {
        GitIdentTime(t.to_owned())
    }
}

impl fmt::Display for GitIdentTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%s %z"))
    }
}

pub trait DateTimeExt<Tz: TimeZone> {
    fn timestamp_micros(&self) -> i64;
}

impl<Tz: TimeZone> DateTimeExt<Tz> for DateTime<Tz> {
    fn timestamp_micros(&self) -> i64 {
        self.timestamp_nanos() / 1000
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Result;
    use chrono::{DateTime, FixedOffset};

    #[test]
    fn test_parse_date() -> Result<()> {
        let data: Vec<(String, DateTime<FixedOffset>)> = vec![
            ("2022-01-02", "2022-01-02T00:00:00-00:00"),
            ("2022-03-05", "2022-03-05T00:00:00-00:00"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), DateTime::parse_from_rfc3339(b).unwrap()))
        .collect();

        for (a, b) in data {
            assert_eq!(*FocusTime::parse_date(a)?, b);
        }

        Ok(())
    }

    #[test]
    fn test_git_time_to_date_time() -> Result<()> {
        let expected_git_offset_mins = -5 * 60;

        let offset = FixedOffset::west(expected_git_offset_mins * 60);

        assert_eq!(offset.utc_minus_local() / 60, expected_git_offset_mins);

        let dt = DateTime::parse_from_rfc3339("2022-02-07T12:34:56-05:00")?;
        let expected_unix_time: i64 = 1644255296;
        let git_time = git2::Time::new(expected_unix_time, expected_git_offset_mins);

        assert_eq!(*FocusTime::from(git_time), dt);

        Ok(())
    }

    #[test]
    fn test_round_trip_focus_time_to_git_time_and_back() -> Result<()> {
        let ft = FocusTime::parse_from_rfc3339("2022-02-07T12:34:56-05:00")?;
        assert_eq!(FocusTime::from(GitTime::from(ft.to_owned())), ft);
        Ok(())
    }

    #[test]
    fn test_focus_time_to_git_time() -> Result<()> {
        let dt = FocusTime::parse_from_rfc3339("2022-02-07T12:34:56-05:00")?;
        let gt = GitTime::new(Time::new(1644255296, -(5 * 60)));
        assert_eq!(GitTime::from(dt), gt);
        Ok(())
    }
}
