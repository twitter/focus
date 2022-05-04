use std::cell::Ref;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;

use super::*;

/// A collection representing projects in a repo
pub struct ProjectSets(FileBackedCollection<ProjectSet>);

impl ProjectSets {
    pub fn new(directory: &dyn AsRef<Path>) -> Result<Self> {
        Ok(Self(FileBackedCollection::<ProjectSet>::new(
            directory,
            OsString::from("projects.json"),
        )?))
    }

    pub fn underlying(&self) -> Ref<HashMap<String, ProjectSet>> {
        self.0.underlying.borrow()
    }
}
