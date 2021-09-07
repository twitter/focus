use std::{collections::HashMap, path::Path, path::PathBuf};

use anyhow::Result;
use internals::repo::ManagedRepo;

pub struct Manager {
    root: PathBuf,
    // repos:
}

impl Manager {
    pub fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.to_owned(),
        })
    }

    pub fn repos() -> HashMap<String, ManagedRepo> {
        todo!("implement")
    }
}
