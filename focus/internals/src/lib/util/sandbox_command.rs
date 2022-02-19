use crate::{app::App, ui::ProgressReporter};
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsStr,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

fn exhibit_file(app: Arc<App>, file: &Path, title: &str) -> Result<()> {
    use std::io;

    let file = File::open(file)?;
    let lines = io::BufReader::new(file).lines();
    let ui = app.ui();
    ui.log("Error", format!("Begin '{}'", title));
    #[allow(clippy::manual_flatten)]
    for line in lines {
        if let Ok(line) = line {
            ui.log("Error", line);
        }
    }
    ui.log("Error", format!("End '{}'", title));

    Ok(())
}

// SandboxCommandRunner is a command that captures stdout and stderr into sandbox logs unless other destinations are specified.
pub struct SandboxCommand {
    app: Arc<App>,
    #[allow(dead_code)]
    progress_reporter: ProgressReporter,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[derive(Debug)]
pub enum SandboxCommandOutput {
    All,
    Stdout,
    Stderr,
    Ignore,
}

impl SandboxCommand {
    pub fn new(
        description: impl Into<String>,
        program: impl AsRef<OsStr>,
        app: Arc<App>,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command = Self::with_command(description.into(), &mut command, app)?;
        Ok((command, sandbox_command))
    }

    pub fn new_with_handles(
        description: impl Into<String>,
        program: impl AsRef<OsStr>,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        app: Arc<App>,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command = Self::with_command_and_handles(
            description.into(),
            &mut command,
            stdin,
            stdout,
            stderr,
            app,
        )?;
        Ok((command, sandbox_command))
    }

    pub fn with_command(description: String, command: &mut Command, app: Arc<App>) -> Result<Self> {
        Self::with_command_and_handles(description, command, None, None, None, app)
    }

    pub fn with_command_and_handles(
        description: String,
        command: &mut Command,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        app: Arc<App>,
    ) -> Result<Self> {
        use std::io::Write;
        let sandbox = app.sandbox();

        // Write the description and get the generated serial to name all the files the same.
        let serial = {
            let (mut description_file, _, serial) = sandbox
                .create_file(Some("sandboxed_command"), Some("script"), None)
                .context("Failed creating description file")?;
            writeln!(description_file, "# {}", description)?;
            write!(
                description_file,
                "{}",
                command.get_program().to_string_lossy()
            )?;
            for arg in command.get_args() {
                write!(description_file, "{}", arg.to_string_lossy())?;
            }
            writeln!(description_file)?;
            serial
        };

        let output_file = |extension: &str| -> Result<(Stdio, PathBuf)> {
            let (file, path, _) =
                sandbox.create_file(Some("sandboxed_command"), Some(extension), Some(serial))?;
            Ok((Stdio::from(file), path))
        };

        let stdin = stdin.unwrap_or_else(Stdio::null);

        let (stdout, stdout_path) = match stdout {
            Some(path) => (Stdio::from(File::open(&path)?), path.to_owned()),
            None => output_file("stdout").context("Failed preparing stdout")?,
        };
        let (stderr, stderr_path) = match stderr {
            Some(path) => (Stdio::from(File::open(&path)?), path.to_owned()),
            None => output_file("stderr").context("Failed preparing stderr")?,
        };

        command.stdin(stdin).stdout(stdout).stderr(stderr);

        let cloned_app_for_progress_reporter = app.clone();
        Ok(Self {
            app,
            progress_reporter: ProgressReporter::new(
                cloned_app_for_progress_reporter,
                description,
            )?,
            stdout_path,
            stderr_path,
        })
    }

    pub fn log(&self, output: SandboxCommandOutput, description: &str) -> Result<()> {
        let title = |o: SandboxCommandOutput| format!("{:?} from {}", o, description);
        let items: Vec<(String, &Path)> = match output {
            SandboxCommandOutput::All => {
                vec![
                    (
                        title(SandboxCommandOutput::Stdout),
                        self.stdout_path.as_path(),
                    ),
                    (
                        title(SandboxCommandOutput::Stderr),
                        self.stderr_path.as_path(),
                    ),
                ]
            }
            SandboxCommandOutput::Stdout => {
                vec![(
                    title(SandboxCommandOutput::Stdout),
                    self.stdout_path.as_path(),
                )]
            }
            SandboxCommandOutput::Stderr => {
                vec![(
                    title(SandboxCommandOutput::Stderr),
                    self.stderr_path.as_path(),
                )]
            }

            SandboxCommandOutput::Ignore => {
                vec![]
            }
        };

        for (title, path) in items {
            exhibit_file(self.app.clone(), path, title.as_str())
                .with_context(|| format!("Exhibiting {}", title))?
        }

        Ok(())
    }

    pub fn read_to_string(
        &self,
        output: SandboxCommandOutput,
        output_string: &mut String,
    ) -> Result<()> {
        use std::io::prelude::*;

        let path = match output {
            SandboxCommandOutput::Stdout => &self.stdout_path,
            SandboxCommandOutput::Stderr => &self.stderr_path,
            _ => bail!("cannot read all outputs into one string"),
        };

        let mut reader = BufReader::new(File::open(path)?);
        reader.read_to_string(output_string)?;
        Ok(())
    }

    pub fn read_buffered(&self, output: SandboxCommandOutput) -> Result<BufReader<File>> {
        let path = match output {
            SandboxCommandOutput::Stdout => &self.stdout_path,
            SandboxCommandOutput::Stderr => &self.stderr_path,
            _ => bail!("cannot read all outputs using one reader"),
        };

        Ok(BufReader::new(File::open(path)?))
    }

    // Run the provided command and if it is not successful, log the process output
    pub fn ensure_success_or_log(
        &self,
        cmd: &mut Command,
        output: SandboxCommandOutput,
        description: &str,
    ) -> Result<ExitStatus> {
        log::debug!("Starting {:?} ({})", cmd, description);
        let mut launch = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn command {}", description))?;

        let program_desc = format!("{:?}", cmd.get_program());
        let tailer = Self::tail(self.app.clone(), &program_desc, &self.stderr_path)
            .context("Could not create log tailer");

        let status = launch
            .wait()
            .with_context(|| format!("Failed to wait for command {}", description))?;
        tailer.iter().for_each(|t| t.stop());
        log::debug!("Command {:?} exited with status {}", cmd, &status);
        if !status.success() {
            self.log(output, description).context("logging output")?;
            bail!("Command {:?} failed: {}", cmd, description);
        }

        Ok(status)
    }

    fn tail(app: Arc<App>, description: &str, path: &Path) -> Result<Tailer> {
        Ok(match File::options().read(true).open(path) {
            Ok(f) => Tailer::new(app, description, f),
            Err(_e) => bail!("Could not open {} for tailing", path.display()),
        })
    }
}

struct Tailer {
    cancel_tx: mpsc::Sender<()>,
    stopped: AtomicBool,
}

impl Tailer {
    pub fn new(app: Arc<App>, description: &str, file: File) -> Self {
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        let description = description.to_owned();
        let _ = std::thread::spawn(move || Self::work(app, description, file, cancel_rx));
        Self {
            cancel_tx,
            stopped: AtomicBool::new(false),
        }
    }

    pub fn stop(&self) {
        if let Ok(updated) =
            self.stopped
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            if updated {
                if let Err(e) = self.cancel_tx.send(()) {
                    log::warn!("Failed to send stop signal to Tailer instance: {}", e);
                }
            }
        }
    }

    fn work(app: Arc<App>, description: String, file: File, cancel_rx: mpsc::Receiver<()>) {
        let desc = format!("{}#stderr", description);
        let buffered_reader = BufReader::new(file);
        let mut lines = buffered_reader.lines();
        while cancel_rx.try_recv().is_err() {
            while let Some(Ok(line)) = lines.next() {
                app.ui().log(desc.clone(), line);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        app.ui().log(desc, "Exited loop");
    }
}

impl Drop for Tailer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn sandboxed_command_capture_all() -> Result<()> {
        let app = Arc::from(App::new(false, false)?);
        let (mut cmd, scmd) = SandboxCommand::new("echo".to_owned(), "echo", app)?;
        cmd.arg("-n").arg("hey").arg("there").status()?;
        let mut output_string = String::new();
        scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)?;
        assert_eq!(output_string, "hey there");

        Ok(())
    }

    #[test]
    fn sandboxed_command_specific_stdin() -> Result<()> {
        let app = Arc::from(App::new(false, false)?);
        let sandbox = app.sandbox();
        let path = {
            let (mut file, path, _) = sandbox.create_file(None, None, None)?;
            file.write_all(b"hello, world")?;
            path
        };
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            "Testing with cat".to_owned(),
            "cat",
            Some(Stdio::from(File::open(&path)?)),
            None,
            None,
            app,
        )?;
        cmd.status()?;
        let mut output_string = String::new();
        scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)?;
        assert_eq!(output_string, "hello, world");

        Ok(())
    }
}
