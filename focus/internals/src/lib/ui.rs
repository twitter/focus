use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier, Mutex,
    },
    thread::{sleep, Builder},
    time::{Duration, Instant},
};

use crate::app::App;

use anyhow::{bail, Context, Result};

pub struct Task {
    description: String,
    current: Mutex<Cell<usize>>,
    total: usize,
    started_at: Instant,
    finished_at: Mutex<Option<Instant>>,
}

impl Task {
    fn new(description: String, current: usize, total: usize) -> Self {
        Self {
            description,
            current: Mutex::new(Cell::new(current)),
            total,
            started_at: Instant::now(),
            finished_at: Mutex::new(None),
        }
    }

    pub fn current(&self) -> usize {
        self.current.lock().expect("lock failed").get()
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn set_finished_at(&self, when: Instant) {
        self.finished_at.lock().expect("lock failed").replace(when);
    }

    pub fn elapsed(&self) -> Duration {
        self.finished_at
            .lock()
            .expect("lock failed")
            .unwrap_or(Instant::now())
            .duration_since(self.started_at)
    }

    pub fn is_done(&self) -> bool {
        self.current() == self.total()
    }

    pub fn update_progress(&self, new: usize) {
        self.current.lock().expect("lock failed").replace(new);
        if new == self.total {
            self.set_finished_at(Instant::now());
        }
    }

    /// Get a reference to the task's description.
    pub fn description(&self) -> &str {
        self.description.as_str()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct TaskHandle {
    id: usize,
}

impl TaskHandle {
    pub fn new(id: usize) -> Self {
        Self { id }
    }
}

pub enum UserInterfaceEvent {
    TaskStateChanged(TaskHandle),
    DiagnosticMessage(String),
}

struct RendererState {
    ui_state: Arc<UserInterfaceState>,
    running: AtomicBool,
}

impl RendererState {
    pub(crate) fn new(ui_state: Arc<UserInterfaceState>, running: AtomicBool) -> Self {
        Self { ui_state, running }
    }

    /// Get a reference to the renderer state's ui state.
    fn ui_state(&self) -> &Arc<UserInterfaceState> {
        &self.ui_state
    }
}
pub struct UserInterfaceRenderer {
    renderer_state: Arc<RendererState>,
    interactive: bool,
    render_thread_exited: Arc<Barrier>,
}

impl UserInterfaceRenderer {
    pub fn new(ui_state: Arc<UserInterfaceState>, interactive: bool) -> Result<Self> {
        let running = AtomicBool::new(true);
        let cloned_ui_state = ui_state.clone();
        let renderer_state = Arc::from(RendererState::new(cloned_ui_state, running));

        let render_thread_exited = Arc::from(Barrier::new(2));
        let cloned_render_thread_exited = render_thread_exited.clone();

        let cloned_render_state = renderer_state.clone();
        Builder::new()
            .name("UI Renderer".to_owned())
            .spawn(move || {
                Self::run_render_thread(cloned_render_state);
                cloned_render_thread_exited.wait();
            })
            .context("Launching the render thread failed")?;

        Ok(Self {
            renderer_state,
            interactive,
            render_thread_exited,
        })
    }

    fn run_render_thread(render_state: Arc<RendererState>) {
        use std::io::Write;
        let ui_state = render_state.ui_state().clone();

        while !ui_state.enabled() {
            // Wait around until we're enabled, or if the thread is told to exit, just leave.
            sleep(Duration::from_millis(50));
            if !render_state.running.load(Ordering::SeqCst) {
                return;
            }
        }

        let mut stdout = std::io::stdout();
        let _stdin = termion::async_stdin();

        let mut highest_task_count = 0usize;

        let update_interval = Duration::from_millis(250);
        let mut last_update = Instant::now() - update_interval; // Always render at first.

        write!(stdout, "{}", termion::clear::All).unwrap();

        while render_state.running.load(Ordering::SeqCst) {
            if last_update.elapsed().lt(&update_interval) {
                sleep(Duration::from_millis(50));
                continue;
            }

            last_update = Instant::now();

            let (screen_width, screen_height) = termion::terminal_size().unwrap();

            let mut active_tasks = Vec::<Arc<Task>>::new();
            if let Ok(locked_ui_state) = ui_state.tasks.lock() {
                for task in locked_ui_state.values() {
                    if !task.is_done() {
                        active_tasks.push(task.clone());
                    }
                }
            }
            if highest_task_count < active_tasks.len() {
                highest_task_count = active_tasks.len();
            }

            write!(stdout, "{}", termion::cursor::Goto(1, 1)).unwrap();
            write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
            let mut used = 0u16;
            let task_status = ui_state.status.lock().unwrap().clone();
            write!(
                stdout,
                "{}focus{} {}{}{}\n\n",
                termion::style::Invert,
                termion::style::NoInvert,
                termion::color::Fg(termion::color::Magenta),
                task_status,
                termion::color::Fg(termion::color::Reset),
            )
            .unwrap(); // On line 3 after this
            used += 2;

            {
                let task_status = format!(
                    "Active Tasks ({} current; {} at peak)",
                    active_tasks.len(),
                    highest_task_count
                );
                used += 1;
                write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
                write!(
                    stdout,
                    "{}{}{}\n",
                    termion::style::Underline,
                    task_status,
                    termion::style::NoUnderline
                )
                .unwrap(); // Line 3

                let area_height = 10;
                // let area_height = active_tasks.len().min(area_height as usize) as u16;
                let mut used_height = 0u16;
                for task in active_tasks.iter().take(area_height as usize) {
                    used_height += 1;
                    write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
                    write!(
                        stdout,
                        "{:>7.2}s {}\n",
                        task.elapsed().as_secs_f32(),
                        task.description()
                    )
                    .unwrap();
                }
                for _ in 0..(area_height - used_height) + 1 {
                    write!(stdout, "{}\n", termion::clear::CurrentLine).unwrap();
                }
                used += area_height;
            }

            write!(stdout, "\n\n").unwrap();
            used += 2;

            {
                let title = "Log Entries";
                used += 1;
                write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
                write!(
                    stdout,
                    "{}{}{}\n",
                    termion::style::Underline,
                    title,
                    termion::style::NoUnderline
                )
                .unwrap(); // Line 3

                let area_height = screen_height - used - 2;

                if let Ok(locked_logs) = ui_state.log_entries.lock() {
                    let mut line_budget = area_height;
                    let mut iterator = locked_logs.iter().rev().take(area_height as usize);
                    while let Some(item) = iterator.next() {
                        if line_budget == 0 {
                            break;
                        }

                        write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
                        let mut header =
                            format!("T +{:>7.1}s ", item.created_at().elapsed().as_secs_f64(),);
                        // Calculate the length of the string without any escape sequences.
                        let stamp = if let Some(time) = item.task_time() {
                            format!(" (in {:.3}s)", time.as_secs_f32())
                        } else {
                            String::new()
                        };
                        let header_len = header.len() + 1 + item.subject().len() + stamp.len() + 1;
                        header.push_str(&format!(
                            "{}{}{}{}{}",
                            termion::color::Fg(termion::color::Green),
                            item.subject(),
                            termion::color::Fg(termion::color::Blue),
                            stamp,
                            termion::color::Fg(termion::color::Reset),
                        ));
                        if header_len > screen_width as usize {
                            break;
                        }

                        let allowed = screen_width as usize - header_len;
                        let mut content = item.content().to_owned();
                        if content.len() > allowed {
                            content.truncate(allowed - 1);
                            content.push('…')
                        }
                        write!(stdout, "{} {}\n", header, content).unwrap();
                        line_budget -= 1;
                    }

                    while line_budget > 0 {
                        line_budget -= 1;
                        write!(stdout, "{}", termion::clear::CurrentLine).unwrap();
                    }
                }
            }

            stdout.flush().unwrap();
        }

        write!(stdout, "\n").unwrap();
        stdout.flush().unwrap();
    }

    pub fn notify(&self, event: UserInterfaceEvent) {
        if let UserInterfaceEvent::DiagnosticMessage(message) = event {
            if self.interactive() {
                // Ignore. It will be listed in the log.
            } else {
                log::info!("{}", message);
            }
        }
    }

    pub fn interactive(&self) -> bool {
        self.interactive
    }

    pub fn stop_and_join(&self) -> Result<()> {
        self.renderer_state.running.store(false, Ordering::SeqCst);
        self.render_thread_exited.wait();
        Ok(())
    }
}

impl Drop for UserInterfaceRenderer {
    fn drop(&mut self) {
        self.stop_and_join()
            .expect("Stopping the render thread failed");
    }
}

#[derive(Debug)]
pub struct LogEntry {
    created_at: Instant,
    subject: String,
    content: String,
    task_time: Option<Duration>,
}

impl LogEntry {
    pub fn new(subject: String, content: String, task_time: Option<Duration>) -> Self {
        Self {
            created_at: Instant::now(),
            subject,
            content,
            task_time,
        }
    }

    /// Get a reference to the log entry's created at.
    pub fn created_at(&self) -> &Instant {
        &self.created_at
    }

    /// Get a reference to the log entry's subject.
    pub fn subject(&self) -> &str {
        self.subject.as_str()
    }

    /// Get a reference to the log entry's contents.
    pub fn content(&self) -> &str {
        self.content.as_str()
    }

    /// Get a reference to the log entry's task time.
    pub fn task_time(&self) -> &Option<Duration> {
        &self.task_time
    }
}

pub struct UserInterfaceState {
    enabled: AtomicBool,
    status: Mutex<String>,
    tasks: Mutex<BTreeMap<TaskHandle, Arc<Task>>>,
    log_entries: Mutex<Vec<LogEntry>>,
    log_entry_limit: usize,
}

impl UserInterfaceState {
    pub fn new(log_entry_limit: usize) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            status: Mutex::new(String::new()),
            tasks: Mutex::new(BTreeMap::new()),
            log_entries: Mutex::new(Vec::new()),
            log_entry_limit,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn new_task(&self, task: Task) -> Result<TaskHandle> {
        match self.tasks.lock() {
            Ok(mut locked_tasks) => {
                let handle = TaskHandle::new(locked_tasks.len());
                locked_tasks.insert(handle, Arc::from(task));
                Ok(handle)
            }
            Err(_) => {
                bail!("Failed to obtain lock")
            }
        }
    }

    pub fn get_task(&self, handle: TaskHandle) -> Result<Option<Arc<Task>>> {
        match self.tasks.lock() {
            Ok(locked_tasks) => {
                if let Some(task) = locked_tasks.get(&handle) {
                    Ok(Some(task.clone()))
                } else {
                    Ok(None)
                }
            }
            Err(_) => {
                bail!("Failed to obtain lock")
            }
        }
    }

    pub fn add_log_entry(&self, entry: LogEntry) {
        match self.log_entries.lock() {
            Ok(mut locked_log_entries) => {
                if locked_log_entries.len() == self.log_entry_limit {
                    locked_log_entries.remove(0);
                }
                locked_log_entries.push(entry);
            }
            Err(e) => {
                log::warn!(
                    "Failed to obtain lock; could not add log entry {:?}: {}",
                    entry,
                    e
                );
            }
        }
    }

    pub fn set_status(&self, status: String) {
        match self.status.lock() {
            Ok(mut locked_status) => {
                *locked_status = status;
            }
            Err(e) => {
                log::warn!(
                    "Failed to obtain lock; could not set status to {}: {}",
                    status,
                    e
                );
            }
        }
    }
}

pub struct UserInterface {
    state: Arc<UserInterfaceState>,
    renderer: UserInterfaceRenderer,
}

impl UserInterface {
    pub fn new(interactive: bool) -> Result<Self> {
        let state = Arc::from(UserInterfaceState::new(100));
        let renderer = UserInterfaceRenderer::new(state.clone(), interactive)
            .context("Failed to start the renderer")?;
        Ok(Self { state, renderer })
    }

    pub fn new_task(&self, task: Task) -> Result<TaskHandle> {
        let cloned_description = task.description.clone();
        let handle = self.state.new_task(task)?;
        self.renderer
            .notify(UserInterfaceEvent::TaskStateChanged(handle));
        self.renderer
            .notify(UserInterfaceEvent::DiagnosticMessage(format!(
                "Started {}",
                cloned_description
            )));
        Ok(handle)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.state.set_enabled(enabled);
    }

    pub fn update_progress(&self, handle: TaskHandle, current: usize) {
        let task = self.state.get_task(handle).expect("Retreiving task failed");
        if let Some(task) = task {
            task.update_progress(current);
            self.renderer
                .notify(UserInterfaceEvent::TaskStateChanged(handle));
            let message = if task.is_done() {
                format!("Finished {}", &task.description)
            } else {
                format!(
                    "[{} / {}] {}",
                    task.current(),
                    task.total(),
                    &task.description
                )
            };
            self.renderer
                .notify(UserInterfaceEvent::DiagnosticMessage(message));
        }
    }

    pub fn elapsed_time_on_task(&self, handle: TaskHandle) -> Result<Duration> {
        let task = self
            .state
            .get_task(handle)
            .context("Retreiving task failed")?;
        if let Some(task) = task {
            return Ok(task.started_at.elapsed());
        } else {
            bail!("Couldn't find task");
        }
    }

    pub fn log(&self, subject: impl Into<String>, content: impl Into<String>) {
        let entry = LogEntry::new(subject.into(), content.into(), None);
        self.state.add_log_entry(entry);
    }

    pub fn log_with_task_time(
        &self,
        subject: String,
        content: String,
        task_time: Option<Duration>,
    ) {
        let entry = LogEntry::new(subject, content, task_time);
        self.state.add_log_entry(entry);
    }

    pub fn status(&self, status: String) {
        self.state.set_status(status);
    }
}

impl Drop for UserInterface {
    fn drop(&mut self) {}
}

pub struct ProgressReporter {
    app: Arc<App>,
    task_handle: TaskHandle,
    description: String,
}

impl ProgressReporter {
    pub fn new(app: Arc<App>, description: String) -> Result<Self> {
        let ui = app.ui();

        let task = Task::new(description.clone(), 0, 1);
        let task_handle = ui
            .new_task(task)
            .context("Registering task with UI failed")?;

        Ok(Self {
            app: app,
            task_handle,
            description,
        })
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        let ui = self.app.ui();
        let task_time = if let Ok(task_time) = ui.elapsed_time_on_task(self.task_handle) {
            Some(task_time)
        } else {
            None
        };
        ui.log_with_task_time("Finished".to_owned(), self.description.clone(), task_time);
        ui.update_progress(self.task_handle, 1);
    }
}
