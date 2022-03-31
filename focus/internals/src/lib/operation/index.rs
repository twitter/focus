use std::path::PathBuf;

use anyhow::Context;

use crate::index::{ObjectDatabase, SimpleGitOdb};

#[derive(
    Clone,
    Debug,
    clap::ArgEnum,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
    strum_macros::EnumIter,
)]
#[strum(serialize_all = "kebab-case")]
pub enum Backend {
    /// Use `SimpleGitOdb` as the back-end. Not for production use.
    Simple,
}

pub fn clear(backend: Backend, sparse_repo: PathBuf) -> anyhow::Result<()> {
    let repo = git2::Repository::open(sparse_repo).context("opening sparse repo")?;
    let odb = match backend {
        Backend::Simple => SimpleGitOdb::new(&repo),
    };
    odb.clear()?;
    Ok(())
}
