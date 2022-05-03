use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use libc::{S_IFREG, S_IRGRP, S_IROTH, S_IRUSR, S_IWUSR};
use tempfile::NamedTempFile;

use crate::message::Message;
use crate::writer::Writer;

const TOOL_INSIGHTS_DIR_DEFAULT: &str = "/opt/twitter_mde/var/log/toolinsights/";
const TOOL_INSIGHTS_DIR_ENV_VAR: &str = "TOOL_INSIGHTS_DIR";

pub struct JsonWriter {
    write_location: PathBuf,
}

impl JsonWriter {
    /// Create a new ToolInsightsWriter with the specified `write_location`.
    /// If no location is specified:
    ///   then write to path specified by `TOOL_INSIGHTS_DIR` environment variable, or
    ///   if environment variable is unset, then write to `TOOL_INSIGHTS_DIR_DEFAULT`.
    pub fn new(write_location: Option<PathBuf>) -> JsonWriter {
        let write_location: PathBuf = match write_location {
            None => {
                let ti_env_var_value = std::env::var(TOOL_INSIGHTS_DIR_ENV_VAR)
                    .unwrap_or_else(|_| TOOL_INSIGHTS_DIR_DEFAULT.to_string());
                let expanded_ti_path = shellexpand::tilde(&ti_env_var_value).to_string();
                PathBuf::from(expanded_ti_path)
            }
            Some(path) => path,
        };

        JsonWriter { write_location }
    }

    // TODO (PFD-161): See if there is a way to write multiple messages to a file
    fn write_data<D>(&self, data: &[D]) -> Result<()>
    where
        D: serde::Serialize,
    {
        for message in data {
            let temporary_file = NamedTempFile::new_in(self.write_location.as_path())
                .context("Failed to create temporary file")?;
            let temporary_file_path = temporary_file.as_ref().to_path_buf();
            let (file, _) = temporary_file.keep()?;
            // temporarily files are created with mode 0o100600, that makes the tool insights daemon
            // fail while trying to process the log because the tool insights daemon is run by
            // the MDE user. We're making the file readable to all to make sure the logs being
            // written can be processed by the tool insights daemon.
            fs::set_permissions(
                temporary_file_path,
                fs::Permissions::from_mode(
                    (S_IFREG | S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH) as u32,
                ),
            )?;
            serde_json::to_writer(file, message).context("Could not write message to file")?;
        }
        Ok(())
    }
}

impl Writer for JsonWriter {
    fn write(&self, messages: &[Message]) -> Result<()> {
        JsonWriter::write_data(self, messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::hashmap;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn ti_json_writer_writes_to_specified_write_location() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let write_location = temp_dir.path().to_owned();
        let writer = JsonWriter::new(Some(write_location));

        let data = hashmap! { "hello".to_string() => vec!["world".to_string()] };
        let serialized_data = serde_json::to_string(&data)?;

        writer.write_data(&[data])?;

        let mut entries = std::fs::read_dir(&temp_dir)?;
        let ti_log = entries.next().unwrap()?.path();
        assert_eq!(std::fs::read_to_string(ti_log.as_path())?, serialized_data);

        let permissions = ti_log.metadata()?.permissions();
        assert_eq!(
            permissions.mode(),
            (S_IFREG | S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH) as u32
        );

        assert!(entries.next().is_none());

        temp_dir.close()?;
        Ok(())
    }
}
