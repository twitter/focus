use crate::sandbox::Sandbox;
use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fmt::{Display, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

pub struct Tool {
    name: String,
    executable: OsString,
}

impl Tool {
    pub fn new(name: &str, executable: &OsStr) -> Result<Self> {
        Ok(Self {
            name: name.to_owned(),
            executable: executable.to_owned(),
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
        let args = args.unwrap_or(&Vec::<OsString>::new()).to_owned();
        let exit_status = Command::new(&self.executable)
            .stdin(stdin_file.unwrap_or(Stdio::null()))
            .stdout(stdout_stdio)
            .stderr(stderr_stdio)
            .args(&args)
            .spawn()
            .with_context(|| format!("spawning {}", self.name))?
            .wait()
            .with_context(|| format!("waiting on {}", self.name))?;

        Ok(InvocationResult {
            name: self.name.clone(),
            binary: self.executable.to_owned(),
            args: args,
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

    fn exhibit_file(file: &Path, title: &str) -> Result<()> {
        use std::fs::File;
        use std::io::{self, BufRead};

        let file = File::open(file)?;
        let lines = io::BufReader::new(file).lines();
        log::info!("--- Begin {} ---", title);
        for line in lines {
            if let Ok(line) = line {
                log::info!("{}", line);
            }
        }
        log::info!("--- End {} ---", title);

        Ok(())
    }

    pub fn log_output(&self) -> Result<()> {
        Self::exhibit_file(self.stdout_path(), &format!("{}: stdout", &self.name))?;
        Self::exhibit_file(self.stderr_path(), &format!("{}: stderr", &self.name))?;
        Ok(())
    }

    pub fn remove_logs(&self) -> Result<()> {
        std::fs::remove_file(&self.stdout_path.as_path()).context("removing stdout file")?;
        std::fs::remove_file(&self.stderr_path.as_path()).context("removing stderr file")?;

        Ok(())
    }

    pub fn or_display_logs(&self) -> Result<()> {
        if !self.exit_status.success() {
            self.log_output().context("displaying logs")?;
            bail!("command failed");
        }

        Ok(())
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
        let original_contents = "hello there\n".as_bytes();
        file.write(original_contents)?;
        file.seek(SeekFrom::Start(0))?;
        file.sync_all()?;
        let cat = Tool::new("cat", OsStr::new("cat"))?;
        let reopened_file = File::open(path).context("reopening file")?;
        let invocation = cat.invoke(None, None, Some(Stdio::from(reopened_file)), &sandbox)?;
        assert!(invocation.or_display_logs().is_ok());
        let mut reader = BufReader::new(File::open(invocation.stdout_path())?);
        let mut contents = Vec::<u8>::new();
        reader.read_to_end(&mut contents)?;
        assert_eq!(contents, original_contents);

        Ok(())
    }

    #[test]
    fn arg_handling() -> Result<()> {
        init_logging();
        let sandbox = Sandbox::new(false)?;
        let original_contents = "fee fie foe fum".as_bytes();
        let echo = Tool::new("echo", OsStr::new("echo"))?;
        let args = vec![
            OsString::from("-n"),
            OsString::from("fee"),
            OsString::from("fie"),
            OsString::from("foe"),
            OsString::from("fum"),
        ];
        let invocation = echo.invoke(Some(&args), None, None, &sandbox)?;
        assert!(invocation.or_display_logs().is_ok());
        let mut reader = BufReader::new(File::open(invocation.stdout_path())?);
        let mut contents = Vec::<u8>::new();
        reader.read_to_end(&mut contents)?;
        assert_eq!(contents, original_contents);

        Ok(())
    }
}
