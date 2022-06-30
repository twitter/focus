// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::app::App;
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use tracing::{debug, debug_span, error, info, info_span, warn};

fn exhibit_file(file: &Path, title: &str) -> Result<()> {
    use std::io;

    let file = File::open(file)?;
    let lines = io::BufReader::new(file).lines();
    error!("Begin {}", &title);
    #[allow(clippy::manual_flatten)]
    for line in lines {
        if let Ok(line) = line {
            error!("{}", &line);
        }
    }
    error!("End {}", &title);

    Ok(())
}

// SandboxCommandRunner is a command that captures stdout and stderr into sandbox logs unless other destinations are specified.
#[derive(Debug, Clone)]
pub struct SandboxCommand {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    git_trace2_path: PathBuf,
    description_path: PathBuf,
}

#[derive(Debug)]
pub enum SandboxCommandOutput {
    All,
    Stdout,
    Stderr,
    Ignore,
    GitTrace2,
}

impl SandboxCommand {
    pub fn new(program: impl AsRef<OsStr>, app: Arc<App>) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command = Self::with_command(&mut command, app)?;
        Ok((command, sandbox_command))
    }

    pub fn new_with_handles(
        program: impl AsRef<OsStr>,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        app: Arc<App>,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command =
            Self::with_command_and_handles(&mut command, stdin, stdout, stderr, app)?;
        Ok((command, sandbox_command))
    }

    pub fn with_command(command: &mut Command, app: Arc<App>) -> Result<Self> {
        Self::with_command_and_handles(command, None, None, None, app)
    }

    pub fn with_command_and_handles(
        command: &mut Command,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        app: Arc<App>,
    ) -> Result<Self> {
        let sandbox = app.sandbox();

        // Write the description and get the generated serial to name all the files the same.
        let (description_path, serial) = {
            let (_, description_path, serial) = sandbox
                .create_file(Some("sandboxed_command"), Some("script"), None)
                .context("Failed creating description file")?;
            (description_path, serial)
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

        let (_git_trace2_file, git_trace2_path) = output_file("git_trace2")?;

        command
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .env("GIT_TRACE2", &git_trace2_path);

        Ok(Self {
            stdout_path,
            stderr_path,
            git_trace2_path,
            description_path,
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
            SandboxCommandOutput::GitTrace2 => {
                vec![(
                    title(SandboxCommandOutput::GitTrace2),
                    self.git_trace2_path.as_path(),
                )]
            }

            SandboxCommandOutput::Ignore => {
                vec![]
            }
        };

        for (title, path) in items {
            exhibit_file(path, title.as_str()).with_context(|| format!("Exhibiting {}", title))?
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
            SandboxCommandOutput::GitTrace2 => &self.git_trace2_path,
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
            SandboxCommandOutput::GitTrace2 => &self.git_trace2_path,
            _ => bail!("cannot read all outputs using one reader"),
        };

        Ok(BufReader::new(File::open(path)?))
    }

    fn pretty_print_command<'cmd>(command: &'cmd mut Command) -> String {
        let convert_os_str =
            |s: &'cmd OsStr| -> &'cmd str { s.to_str().unwrap_or("<???>").trim_matches('"') };

        let mut buf = convert_os_str(command.get_program()).to_owned();
        for arg in command.get_args() {
            buf.push(' ');
            buf.push_str(convert_os_str(arg));
        }
        buf
    }

    // Run the provided command and if it is not successful, log the process output
    pub fn ensure_success_or_log(
        &self,
        cmd: &mut Command,
        output: SandboxCommandOutput,
    ) -> Result<ExitStatus> {
        let command_description = Self::pretty_print_command(cmd);
        let span = debug_span!("Running command", description = %command_description);
        let _guard = span.enter();
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&self.description_path)?;
        writeln!(file, "{}", Self::pretty_print_command(cmd))?;

        let mut launch = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn command {}", &command_description))?;

        let tailer = Self::tail(&command_description, &self.stderr_path)
            .context("Could not create log tailer");

        let status = launch
            .wait()
            .with_context(|| format!("Failed to wait for command {}", &command_description))?;
        tailer.iter().for_each(|t| t.stop());
        debug!(command = %command_description, %status, "Command exited");
        if !status.success() {
            self.log(output, &command_description)
                .context("logging output")?;
            bail!("Command failed: {}", command_description);
        }

        Ok(status)
    }

    fn tail(description: &str, path: &Path) -> Result<Tailer> {
        Ok(match File::options().read(true).open(path) {
            Ok(f) => Tailer::new(description, f),
            Err(_e) => bail!("Could not open {} for tailing", path.display()),
        })
    }
}

struct Tailer {
    cancel_tx: mpsc::Sender<()>,
    stopped: AtomicBool,
}

impl Tailer {
    pub fn new(description: &str, file: File) -> Self {
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        let description = description.to_owned();
        let _ = std::thread::spawn(move || Self::work(description, file, cancel_rx));
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
                    warn!(?e, "Failed to send stop signal to Tailer instance");
                }
            }
        }
    }

    // TODO: Fix waiting for all tailer instances.
    fn work(description: String, file: File, cancel_rx: mpsc::Receiver<()>) {
        let buffered_reader = BufReader::new(file);
        let mut lines = buffered_reader.lines();
        let span = info_span!("Output", command=?description);
        let _guard = span.enter();
        while cancel_rx.try_recv().is_err() {
            while let Some(Ok(line)) = lines.next() {
                info!("{}", line);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Tailer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use focus_testing::init_logging;

    use super::*;
    use anyhow::Result;
    use std::fs::File;
    use std::io::Write;

    #[allow(clippy::redundant_clone)]
    #[test]
    fn sandboxed_command_capture_all() -> Result<()> {
        init_logging();

        let app = Arc::from(App::new_for_testing()?);
        // Make sure to keep the `App` alive until the end of this scope.
        let app = app.clone();
        let (mut cmd, scmd) = SandboxCommand::new("echo", app)?;
        cmd.arg("-n").arg("hey").arg("there").status()?;
        let mut output_string = String::new();
        scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)?;
        assert_eq!(output_string, "hey there");

        Ok(())
    }

    #[test]
    fn sandboxed_command_specific_stdin() -> Result<()> {
        init_logging();

        let app = Arc::from(App::new_for_testing()?);
        let sandbox = app.sandbox();
        let path = {
            let (mut file, path, _) = sandbox.create_file(None, None, None)?;
            file.write_all(b"hello, world")?;
            path
        };
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
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
