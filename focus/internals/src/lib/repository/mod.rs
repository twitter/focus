use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::{app::App, ui::ProgressReporter, util::git_helper};

pub struct Repo {}

impl Repo {
    pub fn read_uuid(repo_path: &Path, app: Arc<App>) -> Result<Uuid> {
        let uuid = {
            if let Some(uuid) =
                git_helper::read_config(repo_path, "twitter.focus.uuid", app.clone())?
            {
                Uuid::from_str(uuid.trim())
                    .context(format!("Could not parse UUID from string '{}'", uuid))?
            } else {
                bail!("Could not read UUID in repo {}", repo_path.display());
            }
        };
        app.ui().log(
            String::from("Repository"),
            format!(
                "Read existing UUID {} for repo at path {}",
                &uuid,
                repo_path.display()
            ),
        );

        Ok(uuid)
    }

    pub fn write_generated_uuid(repo_path: &Path, app: Arc<App>) -> Result<Uuid> {
        let uuid = Uuid::new_v4();
        let _progress = ProgressReporter::new(
            app.clone(),
            format!(
                "Assigning new UUID {} for repo at path {}",
                &uuid,
                repo_path.display()
            ),
        );

        git_helper::write_config(
            repo_path,
            "twitter.focus.uuid",
            uuid.to_string().as_str(),
            app,
        )
        .context("writing generated uuid")?;
        Ok(uuid)
    }
}

pub struct ServedRepo {
    location: PathBuf,
    parent: Option<Arc<Repo>>,
    mutex: Mutex<()>,
}

impl ServedRepo {
    pub fn new(location: &Path, parent: Option<Arc<Repo>>) -> Self {
        Self {
            location: location.to_owned(),
            parent,
            mutex: Mutex::new(()),
        }
    }

    pub fn authoritive(&self) -> bool {
        self.parent.is_none()
    }

    pub fn path(&self) -> &Path {
        self.location.as_path()
    }

    pub fn parent(self) -> Option<Arc<Repo>> {
        if let Some(parent) = self.parent {
            Some(parent.clone())
        } else {
            None
        }
    }

    pub fn seek_to_state() -> Result<()> {
        todo!()
    }
}

pub struct AuthoritiveRepo {}

pub struct RepoManager {
    /// Path to the repositories to be managed.
    repos: PathBuf,
}

impl RepoManager {
    pub fn new(repos: &Path) -> Self {
        Self {
            repos: repos.to_owned(),
        }
    }
}
