mod tests;
pub mod trace;

use std::{io::BufWriter, path::Path};

use anyhow::Result;
use focus_util::time::DateTimeExt;
use git_trace2::{event as gevent, Event as GitEvent};
use heck;
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use std::io::prelude::*;
use strum_macros;
use tempfile::NamedTempFile;

use super::git_trace2;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
/// Simple enum to act as constants for fields we emit
pub(crate) enum JsonField {
    Alias,
    Argv,
    Ancestry,
    Category,
    Cd,
    ChildId,
    ChildClass,
    Code,
    Exe,
    ExecId,
    ExitCode,
    FileLineThread,
    Fmt,
    GitEventName,
    GitVersion,
    Key,
    Hierarchy,
    HookName,
    Label,
    Msg,
    Name,
    Nesting,
    Param,
    Path,
    Pid,
    Ready,
    Sid,
    SidParent,
    Signo,
    StartArgv,
    StatsVersion,
    TAbs,
    ThreadName,
    TRel,
    UseShell,
    Value,
    Worktree,
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    PartialOrd,
    Eq,
    Ord,
    Serialize,
    Deserialize,
    strum_macros::Display,
)]
pub enum Phase {
    #[serde(rename = "B")]
    DurationBegin,
    #[serde(rename = "E")]
    DurationEnd,
    #[serde(rename = "X")]
    Complete,
    #[serde(rename = "i")]
    Instant,
    #[serde(rename = "C")]
    Counter,
    #[serde(rename = "b")]
    NestableStart,
    #[serde(rename = "n")]
    NestableInstant,
    #[serde(rename = "e")]
    NestableEnd,
    #[serde(rename = "s")]
    FlowStart,
    #[serde(rename = "t")]
    FlowStep,
    #[serde(rename = "f")]
    FlowEnd,
    #[serde(rename = "p")]
    Sample,
    #[serde(rename = "N")]
    ObjectCreated,
    #[serde(rename = "O")]
    ObjectSnapshot,
    #[serde(rename = "D")]
    ObjectDestroyed,
    #[serde(rename = "M")]
    Metadata,
    #[serde(rename = "V")]
    MemoryDumpGlobal,
    #[serde(rename = "v")]
    MemoryDumpProcess,
    #[serde(rename = "R")]
    Mark,
    #[serde(rename = "c")]
    ClockSync,
    #[serde(rename = "(")]
    ContextStart,
    #[serde(rename = ")")]
    ContextEnd,
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Complete
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, PartialOrd, Ord)]
pub struct Common {
    pub ts: i64,
    pub pid: u64,
    pub tid: u64,
    pub name: String,
    pub cat: String,
    pub ph: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<i64>,
}

impl Common {
    pub fn set_pid(&mut self, pid: u64) {
        self.pid = pid
    }

    pub fn set_tid(&mut self, tid: u64) {
        self.tid = tid
    }

    pub fn name_for(gitcom: &gevent::Common) -> String {
        let gevent::Common {
            thread, file, line, ..
        } = gitcom;

        format!("{}:{}:{}", file, line, thread)
    }
}

impl From<gevent::Common> for Common {
    fn from(gitcom: gevent::Common) -> Self {
        Self {
            name: Self::name_for(&gitcom),
            ts: gitcom.time.timestamp_micros(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instant {
    #[serde(flatten)]
    pub common: Common,
    pub args: serde_json::Value,
}

impl Instant {
    /// Combines this instant, which should be the earlier instant, with a later instant
    /// to create a Complete. The args of both instants are merged, and the duration is calculated
    /// as the difference between the two timestamp fields on their common
    fn complete(self, mut later: Instant) -> Complete {
        assert!(self.common.ts < later.common.ts);
        let mut c = Complete {
            dur: later.common.ts - self.common.ts,
            ..Complete::from(self)
        };
        c.merge_args(later.args.as_object_mut().unwrap());
        c
    }
}

fn is_git_or_git_exec_cmd<P: AsRef<Path>>(p: P) -> bool {
    p.as_ref()
        .file_stem()
        .and_then(|fs| fs.to_str())
        .map(|name| name.starts_with("git"))
        .unwrap_or(false)
}

fn argv_to_name(argv: Vec<String>) -> String {
    match argv.split_first() {
        Some((first, rest)) if is_git_or_git_exec_cmd(first) => {
            let p: &Path = (first as &str).as_ref();

            match p
                .file_name()
                .and_then(|fp| fp.to_str())
                .map(|s| s.to_owned())
            {
                Some(mut s) => {
                    s.push(' ');
                    s.push_str(&rest.join(" "));
                    s
                }
                None => argv.join(" "),
            }
        }
        _ => argv.join(" "),
    }
}

impl From<gevent::Common> for Instant {
    fn from(gcom: gevent::Common) -> Self {
        Self {
            common: Common {
                ph: Phase::Instant,
                ..Common::from(gcom)
            },
            ..Default::default()
        }
    }
}

impl From<Instant> for Event {
    fn from(i: Instant) -> Self {
        Event::Instant(i)
    }
}

impl Default for Instant {
    fn default() -> Self {
        Self {
            common: Common {
                ph: Phase::Instant,
                ..Default::default()
            },
            args: Default::default(),
        }
    }
}

trait GitEventExt {
    fn event_name(&self) -> String;
    fn into_common(self, phase: Phase) -> Common;
    fn file_line_thread(&self) -> String;
}

impl GitEventExt for GitEvent {
    fn event_name(&self) -> String {
        format!("{}", heck::AsSnakeCase(self.to_string()))
    }

    fn into_common(self, phase: Phase) -> Common {
        match self {
            GitEvent::Version(_) => Common {
                name: "main".into(),
                ph: phase,
                ..Common::from(self.common().clone())
            },

            GitEvent::Start(gevent::Start { common, argv, .. }) => Common {
                name: argv_to_name(argv),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::ChildStart(gevent::ChildStart { common, argv, .. }) => Common {
                name: argv_to_name(argv),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::Exec(gevent::Exec { common, argv, .. }) => Common {
                name: argv_to_name(argv),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::RegionEnter(gevent::RegionEnter {
                common, label, msg, ..
            }) => Common {
                name: label
                    .or_else(|| msg.clone())
                    .unwrap_or_else(|| Common::name_for(&common)),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::Data(gevent::Data {
                common,
                category,
                key,
                value,
                ..
            }) => Common {
                name: format!("data({}): {} -> {}", category, key, value),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::DataJson(gevent::DataJson {
                common,
                category,
                key,
                value,
                ..
            }) => Common {
                name: format!("data({}): {} -> {}", category, key, value),
                ph: phase,
                ..Common::from(common)
            },

            GitEvent::TooManyFiles(_)
            | GitEvent::Exit(_)
            | GitEvent::Atexit(_)
            | GitEvent::Signal(_)
            | GitEvent::Error(_)
            | GitEvent::CmdPath(_)
            | GitEvent::CmdAncestry(_)
            | GitEvent::CmdName(_)
            | GitEvent::CmdMode(_)
            | GitEvent::Alias(_)
            | GitEvent::ChildExit(_)
            | GitEvent::ChildReady(_)
            | GitEvent::ExecResult(_)
            | GitEvent::ThreadStart(_)
            | GitEvent::ThreadExit(_)
            | GitEvent::DefParam(_)
            | GitEvent::DefRepo(_)
            | GitEvent::RegionLeave(_) => Common {
                ph: phase,
                ..Common::from(self.common().clone())
            },
        }
    }

    fn file_line_thread(&self) -> String {
        Common::name_for(self.common())
    }
}

impl From<GitEvent> for Instant {
    fn from(gev: GitEvent) -> Self {
        Self {
            args: Event::chrome_args(&gev),
            common: gev.into_common(Phase::Instant),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complete {
    #[serde(flatten)]
    pub common: Common,
    pub dur: i64,
    pub args: serde_json::Value,
}

impl Complete {
    pub fn merge_args(
        &mut self,
        other_map: &mut serde_json::map::Map<String, serde_json::Value>,
    ) -> &mut Self {
        let args_map = self.args.as_object_mut().unwrap();

        // this cuts down on needless duplication between the begin map and the end map
        let end_map = serde_json::Map::from_iter(
            other_map
                .iter()
                .filter(|(other_k, other_v)| {
                    !args_map.contains_key(*other_k)
                        || match args_map.get(*other_k) {
                            Some(v) => *other_v != v,
                            None => false,
                        }
                })
                .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );

        let mut merged = json!({
            "begin": args_map.clone(),
            "end": end_map,
        });
        args_map.clear();
        args_map.append(
            merged
                .as_object_mut()
                .expect("[BUG] merged wasn't an object"),
        );
        self
    }
}

impl Default for Complete {
    fn default() -> Self {
        Self {
            common: Common {
                ph: Phase::Complete,
                ..Default::default()
            },
            dur: Default::default(),
            args: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

impl From<Complete> for Event {
    fn from(c: Complete) -> Self {
        Event::Complete(c)
    }
}

impl From<Instant> for Complete {
    fn from(i: Instant) -> Self {
        Self {
            common: Common {
                ph: Phase::Complete,
                ..i.common
            },
            dur: Default::default(),
            args: i.args,
        }
    }
}

impl From<GitEvent> for Complete {
    fn from(gev: GitEvent) -> Self {
        Complete::from(Instant::from(gev))
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
)]
#[serde(untagged, rename_all = "snake_case")]
pub enum Event {
    Complete(Complete),
    Instant(Instant),
}

impl Event {
    fn chrome_args(gev: &GitEvent) -> serde_json::Value {
        use JsonField::*;

        impl From<JsonField> for String {
            fn from(jf: JsonField) -> Self {
                format!("{}", heck::AsSnakeCase(jf.to_string()))
            }
        }

        let mut args = match gev {
            GitEvent::Version(gevent::Version {
                common: _,
                evt,
                exe,
            }) => json!({
                StatsVersion: evt,
                GitVersion: exe,
            }),
            GitEvent::TooManyFiles(_) => json!({}),
            GitEvent::Start(gevent::Start {
                common: _,
                t_abs,
                argv,
            }) => json!({
                TAbs: t_abs,
                Argv: argv,
            }),
            GitEvent::Exit(gevent::Exit {
                common: _,
                t_abs,
                code,
            }) => json!({
                TAbs: t_abs,
                Code: code,
            }),
            GitEvent::Atexit(gevent::Atexit {
                common: _,
                t_abs,
                code,
            }) => json!({
                TAbs: t_abs,
                Code: code,
            }),
            GitEvent::Signal(gevent::Signal {
                common: _,
                t_abs,
                signo,
            }) => json!({
                TAbs: t_abs,
                Signo: signo,
            }),
            GitEvent::Error(gevent::Error {
                common: _,
                msg,
                fmt,
            }) => json!({
                Msg: msg,
                Fmt: fmt,
            }),
            GitEvent::CmdPath(gevent::CmdPath { common: _, path }) => json!({
                Path: path,
            }),
            GitEvent::CmdAncestry(gevent::CmdAncestry {
                common: _,
                ancestry,
            }) => json!({
                Ancestry: ancestry,
            }),
            GitEvent::CmdName(gevent::CmdName {
                common: _,
                name,
                hierarchy,
            }) => json!({
                Name: name,
                Hierarchy: hierarchy,
            }),
            GitEvent::CmdMode(gevent::CmdMode { common: _, name }) => json!({
                Name: name,
            }),
            GitEvent::Alias(gevent::Alias {
                common: _,
                alias,
                argv,
            }) => json!({
                Alias: alias,
                Argv: argv,
            }),
            GitEvent::ChildStart(gevent::ChildStart {
                common: _,
                child_id,
                child_class,
                use_shell,
                argv,
                hook_name,
                cd,
            }) => json!({
                ChildId: child_id,
                ChildClass: child_class,
                UseShell: use_shell,
                HookName: hook_name,
                Argv: argv,
                Cd: cd,
            }),
            GitEvent::ChildExit(gevent::ChildExit {
                common: _,
                child_id,
                pid,
                code,
                t_rel,
            }) => json!({
                ChildId: child_id,
                Pid: pid,
                Code: code,
                TRel: t_rel,
            }),
            GitEvent::ChildReady(gevent::ChildReady {
                common: _,
                child_id,
                pid,
                ready,
                t_rel,
            }) => json!({
                ChildId: child_id,
                Pid: pid,
                Ready: ready,
                TRel: t_rel,
            }),
            GitEvent::Exec(gevent::Exec {
                common: _,
                exec_id,
                exe,
                argv,
            }) => json!({
                ExecId: exec_id,
                Exe: exe,
                Argv: argv,
            }),
            GitEvent::ExecResult(gevent::ExecResult {
                common: _,
                exec_id,
                code,
            }) => json!({
                ExecId: exec_id,
                Code: code,
            }),
            GitEvent::ThreadStart(gevent::ThreadStart {
                common: _,
                thread_name,
            }) => json!({
                ThreadName: thread_name,
            }),
            GitEvent::ThreadExit(gevent::ThreadExit {
                common: _,
                thread_name,
                t_rel,
            }) => json!({
                ThreadName: thread_name,
                TRel: t_rel,
            }),
            GitEvent::DefParam(gevent::DefParam {
                common: _,
                param,
                value,
            }) => json!({
                Param: param,
                Value: value,
            }),
            GitEvent::DefRepo(gevent::DefRepo {
                common: _,
                worktree,
            }) => json!({
                Worktree: worktree ,
            }),
            GitEvent::RegionEnter(gevent::RegionEnter {
                common: _,
                nesting,
                category,
                label,
                msg,
            }) => json!({
                Nesting: nesting,
                Category: category,
                Label: label,
                Msg: msg,
            }),
            GitEvent::RegionLeave(gevent::RegionLeave {
                common: _,
                t_rel,
                nesting,
                category,
                label,
                msg,
            }) => json!({
                Nesting: nesting,
                Category: category,
                Label: label,
                Msg: msg,
                TRel: t_rel,
            }),
            GitEvent::Data(gevent::Data {
                common: _,
                t_abs,
                t_rel,
                nesting,
                category,
                key,
                value,
            }) => json!({
                Nesting: nesting,
                Category: category,
                TRel: t_rel,
                TAbs: t_abs,
                Key: key,
                Value: value,
            }),
            GitEvent::DataJson(gevent::DataJson {
                common: _,
                t_abs,
                t_rel,
                nesting,
                category,
                key,
                value,
            }) => json!({
                Nesting: nesting,
                Category: category,
                TRel: t_rel,
                TAbs: t_abs,
                Key: key,
                Value: value,
            }),
        };

        {
            let map = args.as_object_mut().unwrap();
            map.insert(GitEventName.into(), gev.event_name().into());
            map.insert(FileLineThread.into(), gev.file_line_thread().into());
        }

        args
    } // chrome_args

    fn ts(&self) -> i64 {
        match self {
            Event::Complete(c) => c.common.ts,
            Event::Instant(i) => i.common.ts,
        }
    }

    fn common_mut(&mut self) -> &mut Common {
        match self {
            Event::Complete(Complete {
                common,
                dur: _,
                args: _,
            }) => common,
            Event::Instant(Instant { common, args: _ }) => common,
        }
    }

    fn common(&self) -> &Common {
        match self {
            Event::Complete(Complete {
                common,
                dur: _,
                args: _,
            }) => common,
            Event::Instant(Instant { common, args: _ }) => common,
        }
    }

    fn set_pid(&mut self, pid: u64) -> &mut Event {
        self.common_mut().set_pid(pid);
        self
    }

    fn set_tid(&mut self, tid: u64) -> &mut Event {
        self.common_mut().set_tid(tid);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub trace_events: Vec<Event>,
    pub display_time_unit: Option<String>,
    pub system_trace_events: Option<String>,
    pub other_data: serde_json::Value,
}

impl Trace {
    /// Convenience method for writing out a trace as JSON to 'out'.
    /// Will do a buffered write to a tempfile and do an atomic move into place.
    pub fn write_trace_json_to<P: AsRef<Path>>(&self, out: P) -> Result<()> {
        let p = out.as_ref();

        let mut temp = NamedTempFile::new_in(p.parent().ok_or_else(|| {
            anyhow::anyhow!("could not determine parent directoy of output path {:?}", p)
        })?)?;

        serde_json::to_writer(BufWriter::new(temp.as_file_mut()), &self)?;
        temp.flush()?;
        temp.as_file_mut().sync_all()?;
        temp.persist(out)?;

        Ok(())
    }

    fn git_trace_from_file<P: AsRef<Path>>(path: P) -> Result<Trace> {
        let git_events = git_trace2::Events::from_file(path.as_ref())?;
        trace::Builder::from(git_events.into_inner()).build()
    }

    fn git_trace_from_dir<P: AsRef<Path>>(path: P) -> Result<Trace> {
        let events = git_trace2::Events::from_dir(path.as_ref())?;
        trace::Builder::from_iter(events.into_iter().flat_map(|ev| ev.into_inner())).build()
    }

    pub fn git_trace_from<P: AsRef<Path>>(path: P) -> Result<Trace> {
        let p = path.as_ref();
        if p.is_file() {
            Self::git_trace_from_file(path)
        } else if p.is_dir() {
            Self::git_trace_from_dir(path)
        } else {
            Err(anyhow::anyhow!(
                "path {:?} was neither a file or directory",
                p
            ))
        }
    }
}
