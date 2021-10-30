use crate::{app::App, ui::ProgressReporter};
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsStr,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::Arc,
};

fn exhibit_file(app: Arc<App>, file: &Path, title: &str) -> Result<()> {
    use std::io::{self, BufRead};

    let file = File::open(file)?;
    let lines = io::BufReader::new(file).lines();
    let ui = app.ui();
    ui.log(String::from("Error"), format!("Begin '{}'", title));
    for line in lines {
        if let Ok(line) = line {
            ui.log(String::from("Error"), line);
        }
    }
    ui.log(String::from("Error"), format!("End '{}'", title));

    Ok(())
}

// SandboxCommandRunner is a command that captures stdout and stderr into sandbox logs unless other destinations are specified.
pub struct SandboxCommand {
    app: Arc<App>,
    #[allow(unused)]
    progress_reporter: ProgressReporter,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[derive(Debug)]
pub enum SandboxCommandOutput {
    #[allow(unused)]
    All,
    Stdout,
    Stderr,
    #[allow(unused)]
    Ignore,
}

impl SandboxCommand {
    pub fn new<S: AsRef<OsStr>>(
        description: String,
        program: S,
        app: Arc<App>,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command = Self::with_command(description, &mut command, app)?;
        Ok((command, sandbox_command))
    }

    pub fn new_with_handles<S: AsRef<OsStr>>(
        description: String,
        program: S,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        app: Arc<App>,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command =
            Self::with_command_and_handles(description, &mut command, stdin, stdout, stderr, app)?;
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
        let cloned_app = app.clone();
        let sandbox = cloned_app.sandbox();
        let output_file = |extension: &str| -> Result<(Stdio, PathBuf)> {
            let (file, path) = sandbox.create_file(Some("sandboxed_command"), Some(extension))?;
            let mut description_path = path.clone();
            description_path.set_extension("script");
            std::fs::write(&description_path, format!("{:?}", &command).as_bytes())
                .context("writing script description")?;
            Ok((Stdio::from(file), path))
        };
        let stdin = stdin.unwrap_or(Stdio::null());

        let (stdout, stdout_path) = match stdout {
            Some(path) => (Stdio::from(File::open(&path)?), path.to_owned()),
            None => output_file("stdio").context("preparing stdout")?,
        };
        let (stderr, stderr_path) = match stderr {
            Some(path) => (Stdio::from(File::open(&path)?), path.to_owned()),
            None => output_file("stderr").context("preparing stderr")?,
        };

        command.stdin(stdin).stdout(stdout).stderr(stderr);

        let cloned_app_for_progress_reporter = cloned_app.clone();
        Ok(Self {
            app: cloned_app,
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
                .with_context(|| format!("exhibiting {}", title))?
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
        let status = cmd
            .status()
            .with_context(|| format!("launching command {}", description))?;

        log::debug!("Command {:?} exited with status {}", cmd, &status);

        if !status.success() {
            self.log(output, description).context("logging output")?;
            bail!("command {:?} failed: {}", cmd, description);
        }

        Ok(status)
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
            let (mut file, path) = sandbox.create_file(None, None)?;
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