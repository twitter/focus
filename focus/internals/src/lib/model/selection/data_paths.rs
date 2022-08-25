// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use tracing::warn;

use std::path::PathBuf;

use super::*;

pub struct DataPaths {
    pub dot_focus_dir: PathBuf,
    pub focus_dir: PathBuf,
    pub project_dir: PathBuf,
    pub selection_file: PathBuf,
}

impl DataPaths {
    pub fn from_working_tree(working_tree: &WorkingTree) -> Result<Self> {
        let dot_focus_dir = working_tree.work_dir().join(".focus");
        let focus_dir = working_tree.work_dir().join("focus");
        let project_dir = focus_dir.join("projects");
        let selection_file = dot_focus_dir.join("user.selection.json");

        let instance = Self {
            dot_focus_dir,
            focus_dir,
            project_dir,
            selection_file,
        };
        instance
            .ensure_directories_are_set_up_correctly()
            .context("Ensuring directories are set up correctly")?;
        Ok(instance)
    }

    fn ensure_directories_are_set_up_correctly(&self) -> Result<()> {
        if !self.focus_dir.is_dir() {
            warn!(
                focus_dir = ?self.focus_dir,
                "The repo does not have a 'focus' directory at the top of the working tree; no focus configuration will be applied",
            );
        } else if !self.project_dir.is_dir() {
            warn!(
                project_dir = ?self.project_dir,
                "The repo does not have a 'focus/projects' directory at the top of the working tree; no focus projects will be loaded",
            );
        }

        std::fs::create_dir_all(self.dot_focus_dir.as_path())
            .with_context(|| format!("Creating directory {}", &self.dot_focus_dir.display()))?;

        Ok(())
    }
}
