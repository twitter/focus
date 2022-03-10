use std::path::PathBuf;
use std::{env, fs};

use anyhow::{Context, Result};

use crate::tool_insights_message::ToolInsightsMessage;
use crate::util::tmp_filename;

const TOOL_INSIGHTS_DIR_DEFAULT: &str = "/opt/twitter_mde/var/log/toolinsights/";
const TOOL_INSIGHTS_DIR_ENV_VAR: &str = "TOOL_INSIGHTS_DIR";

pub trait ToolInsightsMessageWriter {
    fn write(&self, messages: &[ToolInsightsMessage]) -> Result<()>;
}

pub struct ToolInsightsJsonWriter {
    write_location: PathBuf,
}

impl ToolInsightsJsonWriter {
    /// Create a new ToolInsightsWriter with the specified `write_location`.
    /// If no location is specified:
    ///   then write to path specified by `TOOL_INSIGHTS_DIR` environment variable, or
    ///   if environment variable is unset, then write to `TOOL_INSIGHTS_DIR_DEFAULT`.
    pub fn new(write_location: Option<PathBuf>) -> ToolInsightsJsonWriter {
        let write_location: PathBuf = match write_location {
            None => {
                let ti_env_var_value = env::var(TOOL_INSIGHTS_DIR_ENV_VAR)
                    .unwrap_or_else(|_| TOOL_INSIGHTS_DIR_DEFAULT.to_string());
                let expanded_ti_path = shellexpand::tilde(&ti_env_var_value).to_string();
                PathBuf::from(expanded_ti_path)
            }
            Some(path) => path,
        };

        ToolInsightsJsonWriter { write_location }
    }

    fn write_data<D>(&self, data: &[D]) -> Result<()>
    where
        D: serde::Serialize,
    {
        for message in data {
            let message_string: String =
                serde_json::to_string(message).context("Could not serialize json string")?;
            fs::write(
                tmp_filename(Some(self.write_location.as_path())),
                message_string,
            )
            .context("Could not write tool insights message")?;
        }
        Ok(())
    }
}

impl ToolInsightsMessageWriter for ToolInsightsJsonWriter {
    fn write(&self, messages: &[ToolInsightsMessage]) -> Result<()> {
        ToolInsightsJsonWriter::write_data(self, messages)
    }
}

#[cfg(test)]
mod ti_json_writer_tests {
    use super::*;
    use maplit::hashmap;

    #[test]
    fn ti_json_writer_writes_to_specified_write_location() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let write_location = PathBuf::from(temp_dir.path());
        let writer = ToolInsightsJsonWriter::new(Some(write_location));

        let data = hashmap! { "hello".to_string() => vec!["world".to_string()] };
        let serialized_data = serde_json::to_string(&data)?;

        writer.write_data(&[data])?;

        let mut entries = std::fs::read_dir(&temp_dir)?;

        assert_eq!(
            std::fs::read_to_string(entries.next().unwrap()?.path())?,
            serialized_data
        );
        assert!(entries.next().is_none());

        temp_dir.close()?;
        Ok(())
    }
}
