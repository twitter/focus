use crate::sandbox::Sandbox;
use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::Arc,
};

fn exhibit_file(file: &Path, title: &str) -> Result<()> {
    use std::io::{self, BufRead};

    let file = File::open(file)?;
    let lines = io::BufReader::new(file).lines();
    log::info!("--- Begin {} ---", title);
    for line in lines {
        if let Ok(line) = line {
            log::info!("`[{}]: {}", title, line);
        }
    }
    log::info!("--- End {} ---", title);

    Ok(())
}

// SandboxCommandRunner is a command that captures stdout and stderr into sandbox logs unless other destinations are specified.
pub struct SandboxCommand {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[derive(Debug)]
pub enum SandboxCommandOutput {
    All,
    Stdout,
    Stderr,
}

impl SandboxCommand {
    pub fn new<S: AsRef<OsStr>>(program: S, sandbox: &Sandbox) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command = Self::with_command(&mut command, sandbox)?;
        Ok((command, sandbox_command))
    }

    pub fn new_with_handles<S: AsRef<OsStr>>(
        program: S,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        sandbox: &Sandbox,
    ) -> Result<(Command, Self)> {
        let mut command = Command::new(program);
        let sandbox_command =
            Self::with_command_and_handles(&mut command, stdin, stdout, stderr, sandbox)?;
        Ok((command, sandbox_command))
    }

    pub fn with_command(command: &mut Command, sandbox: &Sandbox) -> Result<Self> {
        Self::with_command_and_handles(command, None, None, None, sandbox)
    }

    pub fn with_command_and_handles(
        command: &mut Command,
        stdin: Option<Stdio>,
        stdout: Option<&Path>,
        stderr: Option<&Path>,
        sandbox: &Sandbox,
    ) -> Result<Self> {
        let output_file = |extension: &str| -> Result<(Stdio, PathBuf)> {
            let (file, path) = sandbox.create_file(Some("sandboxed_command"), Some(extension))?;
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

        Ok(Self {
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
        };

        for (title, path) in items {
            exhibit_file(path, title.as_str()).with_context(|| format!("exhibiting {}", title))?
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
    
    pub fn read_buffered(
        &self,
        output: SandboxCommandOutput,
        
    ) -> Result<BufReader<File>> {
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
        let status = cmd
            .status()
            .with_context(|| format!("launching command {}", description))?;

        log::info!("Command {:?} exited with status {}", cmd, &status);

        if !status.success() {
            self.log(output, description).context("logging output")?;
            bail!("command {:?} failed: {}", cmd, description);
        }

        Ok(status)
    }
}

pub fn os_strings<'a, I>(iter: I) -> Vec<OsString>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut results = Vec::<OsString>::new();
    results.extend(iter.into_iter().map(|s| OsString::from(s)));
    results
}

pub struct Tool {
    name: String,
    executable: OsString,
    prefix_args: Vec<OsString>,
}

impl Tool {
    pub fn new(name: &str, executable: &OsStr, prefix_args: Vec<OsString>) -> Result<Self> {
        Ok(Self {
            name: name.to_owned(),
            executable: executable.to_owned(),
            prefix_args,
        })
    }

    fn captor(name: &str, extension: &str, sandbox: &Sandbox) -> Result<(Stdio, PathBuf)> {
        let (file, path) = sandbox
            .create_file(Some(name), Some(extension))
            .with_context(|| format!("creating file to capture {}", name))?;
        Ok((Stdio::from(file), path))
    }

    pub fn invoke(
        &self,
        args: Option<&Vec<OsString>>,
        dir: Option<&Path>,
        stdin_file: Option<Stdio>,
        sandbox: &Sandbox,
    ) -> Result<InvocationResult> {
        let (stdout_stdio, stdout_path) = Self::captor(&self.name, "stdout", &sandbox)?;
        let (stderr_stdio, stderr_path) = Self::captor(&self.name, "stderr", &sandbox)?;
        let mut arguments = self.prefix_args.clone();
        if let Some(args) = args {
            arguments.extend(args.iter().map(|arg| arg.to_owned()));
        }

        let exit_status = Command::new(&self.executable)
            .stdin(stdin_file.unwrap_or(Stdio::null()))
            .stdout(stdout_stdio)
            .stderr(stderr_stdio)
            .args(&arguments)
            .spawn()
            .with_context(|| format!("spawning {}", self.name))?
            .wait()
            .with_context(|| format!("waiting on {}", self.name))?;

        Ok(InvocationResult {
            name: self.name.clone(),
            binary: self.executable.to_owned(),
            args: arguments,
            exit_status,
            stdout_path,
            stderr_path,
        })
    }
}

pub struct InvocationResult {
    name: String,
    binary: OsString,
    args: Vec<OsString>,
    exit_status: ExitStatus,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl InvocationResult {
    pub fn exit_status(&self) -> &ExitStatus {
        &self.exit_status
    }

    pub fn stdout_path(&self) -> &Path {
        &self.stdout_path.as_path()
    }

    pub fn stderr_path(&self) -> &Path {
        &self.stderr_path.as_path()
    }

    pub fn log_output(&self) -> Result<()> {
        exhibit_file(self.stdout_path(), &format!("{}: stdout", &self.name))?;
        exhibit_file(self.stderr_path(), &format!("{}: stderr", &self.name))?;
        Ok(())
    }

    pub fn remove_logs(&self) -> Result<()> {
        std::fs::remove_file(&self.stdout_path.as_path()).context("removing stdout file")?;
        std::fs::remove_file(&self.stderr_path.as_path()).context("removing stderr file")?;

        Ok(())
    }

    // TODO: Fix this by turning it into a real Try?
    pub fn or_display_logs(&self) -> Result<()> {
        if !self.exit_status.success() {
            self.log_output().context("displaying logs")?;
            bail!("command failed");
        }

        Ok(())
    }

    fn file_contents_trimmed(path: &Path) -> Result<String> {
        use std::io::prelude::*;
        let mut reader = BufReader::new(File::open(path)?);
        let mut output = String::new();
        reader.read_to_string(&mut output)?;
        Ok(output)
    }

    pub fn stdout_trimmed(&self) -> Result<String> {
        Self::file_contents_trimmed(&self.stdout_path())
    }

    pub fn stderr_trimmed(&self) -> Result<String> {
        Self::file_contents_trimmed(&self.stderr_path())
    }
}

impl Display for InvocationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} invocation; binary: {}, args: {:?}, stdout: {}, stderr: {}",
            &self.name,
            &self.binary.to_string_lossy(),
            &self.args,
            &self.stdout_path.display(),
            &self.stderr_path.display(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{prelude::*, BufReader, SeekFrom, Write};

    fn init_logging() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn concatenating() -> Result<()> {
        init_logging();
        let sandbox = Sandbox::new(false)?;
        let (mut file, path) = sandbox.create_file(None, Some("txt"))?;
        let original_contents = "hello there\n";
        file.write(original_contents.as_bytes())?;
        file.seek(SeekFrom::Start(0))?;
        file.sync_all()?;
        let cat = Tool::new("cat", OsStr::new("cat"), vec![])?;
        let reopened_file = File::open(path).context("reopening file")?;
        let invocation = cat.invoke(None, None, Some(Stdio::from(reopened_file)), &sandbox)?;
        assert!(invocation.or_display_logs().is_ok());
        let mut reader = BufReader::new(File::open(invocation.stdout_path())?);
        let mut contents = String::new();
        reader.read_to_string(&mut contents)?;
        assert_eq!(contents, original_contents);

        Ok(())
    }

    #[test]
    fn arg_handling_default_args() -> Result<()> {
        init_logging();
        let sandbox = Sandbox::new(false)?;
        let echo = Tool::new(
            "echo1",
            OsStr::new("echo"),
            vec![OsString::from("-n"), OsString::from("howdy")],
        )?;
        let args = vec![OsString::from("hey"), OsString::from("hello")];
        let invocation = echo.invoke(Some(&args), None, None, &sandbox)?;
        assert!(invocation.or_display_logs().is_ok());
        let mut reader = BufReader::new(File::open(invocation.stdout_path())?);
        let mut contents = String::new();
        reader.read_to_string(&mut contents)?;
        let expected_contents = "howdy hey hello";
        assert_eq!(contents, expected_contents);
        Ok(())
    }

    #[test]
    fn test_os_strings() {
        assert_eq!(
            vec!["foo", "bar", "baz"],
            vec![
                OsString::from("foo"),
                OsString::from("bar"),
                OsString::from("baz")
            ]
        )
    }

    #[test]
    fn sandboxed_command_capture_all() -> Result<()> {
        init_logging();
        let sandbox = Sandbox::new(false)?;
        let (mut cmd, scmd) = SandboxCommand::new("echo", &sandbox)?;
        cmd.arg("-n").arg("hey").arg("there").status()?;
        let mut output_string = String::new();
        scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)?;
        assert_eq!(output_string, "hey there");

        Ok(())
    }

    #[test]
    fn sandboxed_command_specific_stdin() -> Result<()> {
        init_logging();
        let sandbox = Sandbox::new(false)?;
        let path = {
            let (mut file, path) = sandbox.create_file(None, None)?;
            file.write_all(b"hello, world")?;
            path
        };
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            "cat",
            Some(Stdio::from(File::open(&path)?)),
            None,
            None,
            &sandbox,
        )?;
        cmd.status()?;
        let mut output_string = String::new();
        scmd.read_to_string(SandboxCommandOutput::Stdout, &mut output_string)?;
        assert_eq!(output_string, "hello, world");

        Ok(())
    }
}
