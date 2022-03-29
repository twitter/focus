use std::{
    io::{BufRead, BufReader},
    ops::{Deref, DerefMut},
    path::Path,
};

use anyhow::{bail, Result};
use serde_derive::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

use tracing::debug;

#[allow(dead_code)]
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
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Version(event::Version),
    TooManyFiles(event::TooManyFiles),
    Start(event::Start),
    Exit(event::Exit),
    Atexit(event::Atexit),
    Signal(event::Signal),
    Error(event::Error),
    CmdPath(event::CmdPath),
    CmdAncestry(event::CmdAncestry),
    CmdName(event::CmdName),
    CmdMode(event::CmdMode),
    Alias(event::Alias),
    ChildStart(event::ChildStart),
    ChildExit(event::ChildExit),
    ChildReady(event::ChildReady),
    Exec(event::Exec),
    ExecResult(event::ExecResult),
    ThreadStart(event::ThreadStart),
    ThreadExit(event::ThreadExit),
    DefParam(event::DefParam),
    DefRepo(event::DefRepo),
    RegionEnter(event::RegionEnter),
    RegionLeave(event::RegionLeave),
    Data(event::Data),
    #[serde(
        rename(serialize = "data_json", deserialize = "data-json"),
        alias = "data_json"
    )]
    DataJson(event::DataJson),
}

impl Event {
    pub fn common(&self) -> &event::Common {
        match self {
            Event::Version(v) => &v.common,
            Event::TooManyFiles(v) => &v.common,
            Event::Start(v) => &v.common,
            Event::Exit(v) => &v.common,
            Event::Atexit(v) => &v.common,
            Event::Signal(v) => &v.common,
            Event::Error(v) => &v.common,
            Event::CmdPath(v) => &v.common,
            Event::CmdAncestry(v) => &v.common,
            Event::CmdName(v) => &v.common,
            Event::CmdMode(v) => &v.common,
            Event::Alias(v) => &v.common,
            Event::ChildStart(v) => &v.common,
            Event::ChildExit(v) => &v.common,
            Event::ChildReady(v) => &v.common,
            Event::Exec(v) => &v.common,
            Event::ExecResult(v) => &v.common,
            Event::ThreadStart(v) => &v.common,
            Event::ThreadExit(v) => &v.common,
            Event::DefParam(v) => &v.common,
            Event::DefRepo(v) => &v.common,
            Event::RegionEnter(v) => &v.common,
            Event::RegionLeave(v) => &v.common,
            Event::Data(v) => &v.common,
            Event::DataJson(v) => &v.common,
        }
    }

    pub fn sid(&self) -> &event::Sid {
        &self.common().sid
    }

    // this method doesn't do anything functional, however breaking these
    // 26 different Event wrappers out each time gets tedious, so this is
    // here to use as a copy-pasta basis when you need to do matching and
    // destructuring binds on Event.
    //
    // it's defined as a function so that the compiler will complain if/when
    // the definitions get updated
    #[allow(unused_variables)]
    fn _boilerplate(&self) {
        match self {
            Event::Version(event::Version { common, evt, exe }) => todo!(),
            Event::TooManyFiles(event::TooManyFiles { common }) => todo!(),
            Event::Start(event::Start {
                common,
                t_abs,
                argv,
            }) => todo!(),
            Event::Exit(event::Exit {
                common,
                t_abs,
                code,
            }) => todo!(),
            Event::Atexit(event::Atexit {
                common,
                t_abs,
                code,
            }) => todo!(),
            Event::Signal(event::Signal {
                common,
                t_abs,
                signo,
            }) => todo!(),
            Event::Error(event::Error { common, msg, fmt }) => todo!(),
            Event::CmdPath(event::CmdPath { common, path }) => todo!(),
            Event::CmdAncestry(event::CmdAncestry { common, ancestry }) => todo!(),
            Event::CmdName(event::CmdName {
                common,
                name,
                hierarchy,
            }) => todo!(),
            Event::CmdMode(event::CmdMode { common, name }) => todo!(),
            Event::Alias(event::Alias {
                common,
                alias,
                argv,
            }) => todo!(),
            Event::ChildStart(event::ChildStart {
                common,
                child_id,
                child_class,
                use_shell,
                argv,
                hook_name,
                cd,
            }) => todo!(),
            Event::ChildExit(event::ChildExit {
                common,
                child_id,
                pid,
                code,
                t_rel,
            }) => todo!(),
            Event::ChildReady(event::ChildReady {
                common,
                child_id,
                pid,
                ready,
                t_rel,
            }) => todo!(),
            Event::Exec(event::Exec {
                common,
                exec_id,
                exe,
                argv,
            }) => todo!(),
            Event::ExecResult(event::ExecResult {
                common,
                exec_id,
                code,
            }) => todo!(),
            Event::ThreadStart(event::ThreadStart {
                common,
                thread_name,
            }) => todo!(),
            Event::ThreadExit(event::ThreadExit {
                common,
                thread_name,
                t_rel,
            }) => todo!(),
            Event::DefParam(event::DefParam {
                common,
                param,
                value,
            }) => todo!(),
            Event::DefRepo(event::DefRepo { common, worktree }) => todo!(),
            Event::RegionEnter(event::RegionEnter {
                common,
                nesting,
                category,
                label,
                msg,
            }) => todo!(),
            Event::RegionLeave(event::RegionLeave {
                common,
                t_rel,
                nesting,
                category,
                label,
                msg,
            }) => todo!(),
            Event::Data(event::Data {
                common,
                t_abs,
                t_rel,
                nesting,
                category,
                key,
                value,
            }) => todo!(),
            Event::DataJson(event::DataJson {
                common,
                t_abs,
                t_rel,
                nesting,
                category,
                key,
                value,
            }) => todo!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Events(Vec<Event>);

impl Events {
    pub fn from_file(path: &Path) -> Result<Events> {
        if !path.is_file() {
            bail!("path {:?} was not a file", path);
        }

        let mut result: Vec<Event> = Vec::new();

        let bytes = std::fs::read(&path)?;
        debug!("parsing: {:?}", &path);
        for line in BufReader::new(bytes.as_slice()).lines() {
            let s = line?;
            let event: Event = serde_json::from_str(&s)?;
            result.push(event);
        }

        Ok(Events(result))
    }

    fn is_hidden(entry: &DirEntry) -> bool {
        entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or(false)
    }

    pub fn file_iter(path: &Path) -> impl Iterator<Item = DirEntry> {
        WalkDir::new(path)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|ent| !Self::is_hidden(ent))
            .filter_map(|e| e.ok())
    }

    pub fn from_dir(path: &Path) -> Result<Vec<Events>> {
        Self::file_iter(path)
            .filter(|dirent| dirent.path().is_file())
            .map(|dirent| Self::from_file(dirent.path()))
            .collect::<Result<Vec<Events>>>()
    }

    pub fn into_inner(self) -> Vec<Event> {
        let Self(inner) = self;
        inner
    }
}

impl Deref for Events {
    type Target = Vec<Event>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Events {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsMut<Vec<Event>> for Events {
    fn as_mut(&mut self) -> &mut Vec<Event> {
        &mut self.0
    }
}

pub mod event {
    use chrono::{DateTime, TimeZone, Utc};
    use std::ops::Deref;
    use std::path::PathBuf;

    use serde_derive::{Deserialize, Serialize};

    #[derive(
        Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash, Default,
    )]
    #[serde(transparent)]
    pub struct Sid(String);

    impl Sid {
        pub fn new<S: Into<String>>(s: S) -> Self {
            Sid(s.into())
        }

        pub fn parent(&self) -> Option<&str> {
            match self.0.split_once('/') {
                Some((pfx, _)) => Some(pfx),
                None => None,
            }
        }

        pub fn child(&self) -> &str {
            match self.0.split_once('/') {
                Some((_, sufx)) => sufx,
                None => &self.0,
            }
        }
    }

    impl Deref for Sid {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsRef<str> for Sid {
        fn as_ref(&self) -> &str {
            self.deref()
        }
    }

    // a simple macro for defining a mapping from an instance of one
    // of these structs back to their associated Event variant.
    macro_rules! from_enum_impl {
        ($ident:ident) => {
            impl From<$ident> for $crate::git_trace2::Event {
                fn from(t: $ident) -> $crate::git_trace2::Event {
                    $crate::git_trace2::Event::$ident(t)
                }
            }
        };
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct Common {
        pub sid: Sid,
        pub thread: String,
        pub time: DateTime<Utc>,
        pub file: String,
        pub line: u64,
        pub repo: Option<i64>,
    }

    impl Default for Common {
        fn default() -> Self {
            Self {
                sid: Default::default(),
                thread: Default::default(),
                time: Utc.timestamp(0, 0),
                file: Default::default(),
                line: Default::default(),
                repo: Default::default(),
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Version {
        #[serde(flatten)]
        pub common: Common,
        pub evt: String,
        pub exe: String,
    }
    from_enum_impl!(Version);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct TooManyFiles {
        #[serde(flatten)]
        pub common: Common,
    }
    from_enum_impl!(TooManyFiles);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Start {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub argv: Vec<String>,
    }
    from_enum_impl!(Start);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Exit {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub code: i64,
    }
    from_enum_impl!(Exit);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Atexit {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub code: i64,
    }
    from_enum_impl!(Atexit);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Signal {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub signo: i64,
    }
    from_enum_impl!(Signal);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Error {
        #[serde(flatten)]
        pub common: Common,
        pub msg: String,
        pub fmt: String,
    }
    from_enum_impl!(Error);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct CmdPath {
        #[serde(flatten)]
        pub common: Common,
        pub path: String,
    }
    from_enum_impl!(CmdPath);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct CmdAncestry {
        #[serde(flatten)]
        pub common: Common,
        pub ancestry: Vec<String>,
    }
    from_enum_impl!(CmdAncestry);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct CmdName {
        #[serde(flatten)]
        pub common: Common,
        pub name: String,
        pub hierarchy: String,
    }
    from_enum_impl!(CmdName);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct CmdMode {
        #[serde(flatten)]
        pub common: Common,
        pub name: String,
    }
    from_enum_impl!(CmdMode);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Alias {
        #[serde(flatten)]
        pub common: Common,
        pub alias: String,
        pub argv: Vec<String>,
    }
    from_enum_impl!(Alias);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ChildStart {
        #[serde(flatten)]
        pub common: Common,
        pub child_id: u64,
        pub child_class: String,
        pub use_shell: bool,
        pub argv: Vec<String>,
        pub hook_name: Option<String>,
        pub cd: Option<PathBuf>,
    }
    from_enum_impl!(ChildStart);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ChildExit {
        #[serde(flatten)]
        pub common: Common,
        pub child_id: u64,
        pub pid: i64,
        pub code: i64,
        pub t_rel: f64,
    }
    from_enum_impl!(ChildExit);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ChildReady {
        #[serde(flatten)]
        pub common: Common,
        pub child_id: u64,
        pub pid: i64,
        pub ready: String,
        pub t_rel: f64,
    }
    from_enum_impl!(ChildReady);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Exec {
        #[serde(flatten)]
        pub common: Common,
        pub exec_id: u64,
        pub exe: String,
        pub argv: Vec<String>,
    }
    from_enum_impl!(Exec);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ExecResult {
        #[serde(flatten)]
        pub common: Common,
        pub exec_id: u64,
        pub code: i64,
    }
    from_enum_impl!(ExecResult);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ThreadStart {
        #[serde(flatten)]
        pub common: Common,
        pub thread_name: String,
    }
    from_enum_impl!(ThreadStart);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct ThreadExit {
        #[serde(flatten)]
        pub common: Common,
        pub thread_name: String,
        pub t_rel: f64,
    }
    from_enum_impl!(ThreadExit);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct DefParam {
        #[serde(flatten)]
        pub common: Common,
        pub param: String,
        pub value: String,
    }
    from_enum_impl!(DefParam);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct DefRepo {
        #[serde(flatten)]
        pub common: Common,
        pub worktree: PathBuf,
    }
    from_enum_impl!(DefRepo);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct RegionEnter {
        #[serde(flatten)]
        pub common: Common,
        pub nesting: u64,
        pub category: Option<String>,
        pub label: Option<String>,
        pub msg: Option<String>,
    }
    from_enum_impl!(RegionEnter);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct RegionLeave {
        #[serde(flatten)]
        pub common: Common,
        pub t_rel: f64,
        pub nesting: u64,
        pub category: Option<String>,
        pub label: Option<String>,
        pub msg: Option<String>,
    }
    from_enum_impl!(RegionLeave);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct Data {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub t_rel: f64,
        pub nesting: u64,
        pub category: String,
        pub key: String,
        pub value: String,
    }
    from_enum_impl!(Data);

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    pub struct DataJson {
        #[serde(flatten)]
        pub common: Common,
        pub t_abs: f64,
        pub t_rel: f64,
        pub nesting: u64,
        pub category: String,
        pub key: String,
        pub value: serde_json::Value,
    }
    from_enum_impl!(DataJson);
}

#[cfg(test)]
mod tests {

    use super::*;
    use anyhow::{bail, Result};
    use serde_json;

    pub(super) mod data {
        use std::collections::HashMap;

        #[rustfmt::skip]
        pub(in super) const DATA: &[(&str, &str)] = &[
            ("version",        r#"{ "event":"version", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "evt":"3", "exe":"2.20.1.155.g426c96fcdb" }"#),
            ("too_many_files", r#"{ "event":"too_many_files", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42 }"#),
            ("start",          r#"{ "event":"start", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "t_abs":0.001227, "argv":["git", "version"] }"#),
            ("exit",           r#"{ "event":"exit", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "t_abs":0.001227, "code":0 }"#),
            ("atexit",         r#"{ "event":"atexit", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "t_abs":0.001227, "code":0 }"#),
            ("signal",         r#"{ "event":"signal", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "t_abs":0.001227, "signo":13 }"#),
            ("error",          r#"{ "event":"error", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "msg":"invalid option: --cahced", "fmt":"invalid option: %s" }"#),
            ("cmd_path",       r#"{ "event":"cmd_path", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "path":"/usr/bin/git" }"#),
            ("cmd_ancestry",   r#"{ "event":"cmd_ancestry", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "ancestry":["bash","tmux: server","systemd"] }"#),
            ("cmd_name",       r#"{ "event":"cmd_name", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "name":"pack-objects", "hierarchy":"push/pack-objects" }"#),
            ("cmd_mode",       r#"{ "event":"cmd_mode", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "name":"branch" }"#),
            ("alias",          r#"{ "event":"alias", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "alias":"1", "argv":["log", "--graph"] }"#),
            ("child_start",    r#"{ "event":"child_start", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "child_id":2, "child_class":"?", "use_shell":false, "argv":["git","rev-list","--objects","--stdin","--not","--all","--quiet"], "hook_name":"<hook_name>", "cd":"<path>" }"#),
            ("child_exit",     r#"{ "event":"child_exit", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "child_id":2, "pid":14708, "code":0, "t_rel":0.110605 }"#),
            ("child_ready",    r#"{ "event":"child_ready", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "child_id":2, "pid":14708, "ready":"ready", "t_rel":0.110605 }"#),
            ("exec",           r#"{ "event":"exec", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "exec_id":0, "exe":"git", "argv":["foo", "bar"] }"#),
            ("exec_result",    r#"{ "event":"exec_result", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "exec_id":0, "code":1 }"#),
            ("thread_start",   r#"{ "event":"thread_start", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "thread_name":"th02:preload_thread" }"#),
            ("thread_exit",    r#"{ "event":"thread_exit", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "thread_name":"th02:preload_thread", "t_rel":0.007328 }"#),
            ("def_param",      r#"{ "event":"def_param", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "param":"core.abbrev", "value":"7" }"#),
            ("def_repo",       r#"{ "event":"def_repo", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42,  "repo":1, "worktree":"/Users/jeffhost/work/gfw" }"#),
            ("region_enter",   r#"{ "event":"region_enter", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42,  "repo":1, "nesting":1, "category":"index", "label":"do_read_index", "msg":".git/index" }"#),
            ("region_leave",   r#"{ "event":"region_leave", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42,  "repo":1, "t_rel":0.002876, "nesting":1, "category":"index", "label":"do_read_index", "msg":".git/index" }"#),
            ("data",           r#"{ "event":"data", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42,  "repo":1, "t_abs":0.024107, "t_rel":0.001031, "nesting":2, "category":"index", "key":"read/cache_nr", "value":"3552" }"#),
            ("data-json",      r#"{ "event":"data-json", "sid":"20190408T191827.272759Z-H9b68c35f-P00003510", "thread":"main", "time":"2019-04-08T19:18:27.282761Z", "file":"common-main.c", "line":42, "repo":1, "t_abs":0.015905, "t_rel":0.015905, "nesting":1, "category":"process", "key":"windows/ancestry", "value":["bash.exe","bash.exe"] }"#),
        ];

        pub(super) fn hashmap() -> HashMap<String, String> {
            HashMap::from_iter(DATA.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())))
        }
    }

    #[test]
    fn test_parse_examples() -> Result<()> {
        for (k, v) in data::DATA.iter() {
            let xyz: serde_json::Result<Event> = serde_json::from_str(*v);
            if let Err(e) = xyz {
                println!("failure: {}, {:?}", *k, e);
                bail!(e)
            }
        }

        Ok(())
    }

    #[test]
    // make sure the round-trip produces the correctly escaped name for "data-json"
    fn test_data_json_serialize_name() -> Result<()> {
        let hm = data::hashmap();
        let dj_str: &str = hm.get("data-json").unwrap();

        let v1: Event = serde_json::from_str(dj_str)?;
        let roundtrip = serde_json::to_string(&v1)?;
        let v2: serde_json::Value = serde_json::from_str(&roundtrip)?;

        assert_eq!(serde_json::to_value(&v1)?, v2);

        Ok(())
    }

    #[test]
    fn test_from_definition_macro_works() -> Result<()> {
        let v = event::Version::default();
        if let Event::Version(event::Version { .. }) = v.into() {
        } else {
            panic!("wtf?! wrong type returned from into")
        };
        Ok(())
    }
}
