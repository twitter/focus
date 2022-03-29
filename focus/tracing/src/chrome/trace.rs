use crate::{
    chrome::{Complete, Trace},
    git_trace2::{event as gevent, event::Sid, Event as GitEvent},
};
use anyhow::Result;
use std::{cell::Cell, collections::HashMap};

use super::{Event, Instant};

const MICROS_PER_SEC: i64 = 1000000;

#[derive(Debug, Clone)]
struct SidPidMapper {
    next_pid: u64,
    map: HashMap<String, u64>,
}

impl Default for SidPidMapper {
    fn default() -> Self {
        Self {
            next_pid: 1,
            map: Default::default(),
        }
    }
}

impl SidPidMapper {
    pub fn get<S: AsRef<str>>(&mut self, sid: S) -> u64 {
        let sid = sid.as_ref();
        match self.map.get(sid) {
            Some(pid) => *pid,
            None => {
                let pid = self.next_pid;
                self.next_pid += 1;
                self.map.insert(sid.to_owned(), pid);
                pid
            }
        }
    }
}

// the events of a single git "session"
struct Session {
    pub git_events: Vec<GitEvent>,
    pub sid: Sid,
    pub pid: u64,

    version: Cell<Option<Instant>>,
    start: Cell<Option<Instant>>,
    region_enter: Vec<Instant>,
    child_start: Vec<Instant>,
    thread_start: Vec<Instant>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            git_events: Default::default(),
            sid: Sid::new(""), // this is gross, need to make sure this is set before serializing
            pid: Default::default(),
            version: Default::default(),
            start: Default::default(),
            region_enter: Default::default(),
            child_start: Default::default(),
            thread_start: Default::default(),
        }
    }
}

impl Session {
    pub fn add_git_event(&mut self, event: GitEvent) {
        self.git_events.push(event);
    }

    fn push_complete(
        start: Instant,
        end: Instant,
        pid: u64,
        thread_id: Option<u64>,
        res: &mut Vec<Event>,
    ) {
        res.push(Self::complete_event(start, end, pid, thread_id));
    }

    /// Merge two Instant events together (start and end), update their PID, and optionally their
    /// thread id
    fn complete_event(start: Instant, end: Instant, pid: u64, thread_id: Option<u64>) -> Event {
        let mut event = Event::from(start.complete(end));
        event.set_pid(pid);
        if let Some(tid) = thread_id {
            event.set_tid(tid);
        }
        event
    }

    fn push_event(pid: u64, mut event: Event, res: &mut Vec<Event>) {
        event.set_pid(pid);
        res.push(event);
    }

    pub fn build(mut self) -> Vec<Event> {
        assert!(!self.sid.is_empty());

        let mut res: Vec<Event> = Vec::new();

        for gev in self.git_events.into_iter() {
            match gev {
                GitEvent::Version(_) => {
                    let prev = self.version.replace(Some(Instant::from(gev)));
                    assert!(prev.is_none(), "[BUG] expected only one Version event");
                }
                GitEvent::Atexit(_) => {
                    Self::push_complete(
                        self.version
                            .replace(None)
                            .expect("[BUG] expected got AtExit before Version"),
                        Instant::from(gev),
                        self.pid,
                        None,
                        &mut res,
                    );
                }
                GitEvent::Start(_) => {
                    let prev = self.start.replace(Some(Instant::from(gev)));
                    assert!(prev.is_none(), "[BUG] expected to see only one Start event");
                }
                GitEvent::Exit(_) => {
                    Self::push_complete(
                        self.start
                            .replace(None)
                            .expect("[BUG] Exit encountered before Start event"),
                        Instant::from(gev),
                        self.pid,
                        None,
                        &mut res,
                    );
                }

                GitEvent::RegionEnter(_) => self.region_enter.push(Instant::from(gev)),
                GitEvent::RegionLeave(_) => {
                    let thread_id = match &gev {
                        GitEvent::RegionLeave(rleave) => rleave.nesting,
                        v => panic!("[BUG] expected RegionLeave event, got: {:?}", v),
                    };

                    Self::push_complete(
                        self.region_enter
                            .pop()
                            .expect("[BUG] expected Some(RegionEnter)"),
                        Instant::from(gev),
                        self.pid,
                        Some(thread_id),
                        &mut res,
                    );
                }

                GitEvent::ChildStart(_) => self.child_start.push(Instant::from(gev)),
                GitEvent::ChildExit(_) => {
                    Self::push_complete(
                        self.child_start
                            .pop()
                            .expect("[BUG] expected Some(ChildStart)"),
                        Instant::from(gev),
                        self.pid,
                        None,
                        &mut res,
                    );
                }

                GitEvent::ThreadStart(_) => self.child_start.push(Instant::from(gev)),
                GitEvent::ThreadExit(_) => {
                    Self::push_complete(
                        self.thread_start
                            .pop()
                            .expect("[BUG] expected Some(ThreadStart)"),
                        Instant::from(gev),
                        self.pid,
                        None,
                        &mut res,
                    );
                }
                ref gev @ GitEvent::Data(gevent::Data { t_rel, nesting, .. })
                | ref gev @ GitEvent::DataJson(gevent::DataJson { t_rel, nesting, .. }) => {
                    let mut c = Complete::from(gev.clone());
                    // calculates the duration of this event from the t_rel field
                    c.dur = (t_rel * MICROS_PER_SEC as f64) as i64;

                    // ok this is super super gross, but "data" seems to be emitted at
                    // the end of the work done (so its time is at the end of work) but
                    // it really started at (time - t_rel). WE make the adjustment here
                    // because it "looks right"
                    c.common.ts -= (t_rel * MICROS_PER_SEC as f64) as i64;

                    // using the thread id like this allows us to have another "lane"
                    // where the supplemental thread and region specific info can be displayed
                    c.common.tid = nesting;

                    Self::push_event(self.pid, c.into(), &mut res);
                }

                GitEvent::Signal(_)
                | GitEvent::Error(_)
                | GitEvent::CmdPath(_)
                | GitEvent::TooManyFiles(_)
                | GitEvent::CmdAncestry(_)
                | GitEvent::CmdName(_)
                | GitEvent::CmdMode(_)
                | GitEvent::Alias(_)
                | GitEvent::ChildReady(_)
                | GitEvent::Exec(_)
                | GitEvent::ExecResult(_)
                | GitEvent::DefParam(_)
                | GitEvent::DefRepo(_) => {
                    Self::push_event(self.pid, Instant::from(gev).into(), &mut res);
                }
            }
        }

        // ok, now we clean up all the stupid things that git didn't actually
        // emit as pairs because it's totally awesome

        let vs: &[Instant] = &[
            self.version.replace(None).as_slice(),
            self.start.replace(None).as_slice(),
            self.region_enter.as_ref(),
            self.thread_start.as_ref(),
        ]
        .concat();

        for v in vs {
            Self::push_event(self.pid, v.clone().into(), &mut res);
        }

        res
    }
}

trait AsSlice<T> {
    fn as_slice(&self) -> &[T];
}

// allows us to view an Option<T> as a &[T]
impl<T> AsSlice<T> for Option<T> {
    fn as_slice(&self) -> &[T] {
        self.as_ref().map(core::slice::from_ref).unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub struct Builder {
    git_events: Vec<GitEvent>,
}

impl Builder {
    pub fn add_events<V: AsMut<Vec<GitEvent>>>(&mut self, mut events: V) -> &mut Self {
        self.git_events.append(events.as_mut());
        self
    }

    pub fn add_event(&mut self, event: GitEvent) -> &mut Self {
        self.git_events.push(event);
        self
    }

    fn relativize_timestamps(events: &mut Vec<Event>) {
        if let Some(min) = events.iter().min_by_key(|ev| ev.ts()) {
            let min_ts = min.ts();
            for ev in events.iter_mut() {
                let common = ev.common_mut();
                common.ts -= min_ts;
            }
        }
    }

    fn sort_events(events: &mut Vec<Event>) {
        events.sort_by(|a, b| a.common().cmp(b.common()))
    }

    /// Take all git events added to this Builder and create Session instances for them.
    /// Each Session instance will contain the events for a single git "sid" which is
    /// equivalent to a single git process.
    fn into_sessions(self) -> Vec<Session> {
        let mut spmap = SidPidMapper::default();

        let map: HashMap<Sid, Session> = self
            .git_events
            .into_iter()
            .map(|gev| {
                let sid = gev.sid().clone();
                (sid, gev)
            })
            .fold(HashMap::new(), |mut map, (sid, gev)| {
                match map.get_mut(&sid) {
                    Some(session) => {
                        session.add_git_event(gev);
                        map
                    }
                    None => {
                        let mut session = Session {
                            pid: spmap.get(&sid),
                            sid: sid.to_owned(),
                            ..Session::default()
                        };
                        session.add_git_event(gev);
                        map.insert(sid.to_owned(), session);
                        map
                    }
                }
            });

        map.into_values().collect()
    }

    pub fn build(self) -> Result<Trace> {
        use rayon::prelude::*;

        let sessions: Vec<Session> = self.into_sessions();

        let mut events: Vec<Event> = sessions
            .into_par_iter()
            .map(|session| session.build())
            .flatten()
            .collect();

        Self::relativize_timestamps(&mut events);
        Self::sort_events(&mut events);

        Ok(Trace {
            trace_events: events,
            ..Default::default()
        })
    }
}

impl From<Vec<GitEvent>> for Builder {
    fn from(events: Vec<GitEvent>) -> Self {
        let mut builder = Builder::default();
        builder.add_events(events);
        builder
    }
}

impl FromIterator<GitEvent> for Builder {
    fn from_iter<T: IntoIterator<Item = GitEvent>>(iter: T) -> Self {
        let mut builder = Builder::default();
        for event in iter {
            builder.add_event(event);
        }
        builder
    }
}
