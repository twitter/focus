use std::{
    collections::HashMap,
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

    #[allow(unused)]
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

    pub fn ensure_has_content(_branch: &str, _commit_id: git2::Oid) -> Result<()> {
        todo!()
    }
}

pub struct AuthoritiveRepo {}

#[allow(unused)]
pub struct RepoManager {
    /// Path to the repositories to be managed.
    path: PathBuf,

    repos: Arc<Mutex<HashMap<Uuid, ServedRepo>>>,
}

impl RepoManager {
    pub fn new(path: &Path, app: Arc<App>) -> Result<Self> {
        let repos = Arc::new(Mutex::new(Self::scan(path, app.clone())?));
        Ok(Self {
            path: path.to_owned(),
            repos,
        })
    }

    fn scan(path: &Path, app: Arc<App>) -> Result<HashMap<Uuid, ServedRepo>> {
        let mut result = HashMap::new();

        if !path.is_dir() {
            bail!("{} is not a directory", path.display());
        }

        let mut directory_reader = std::fs::read_dir(path)?;

        while let Some(entry) = directory_reader.next() {
            let cloned_app = app.clone();
            match entry {
                Ok(directory_entry) => {
                    let repo_path = directory_entry.path();
                    let git_dir = repo_path.join(".git");
                    if !git_dir.is_dir() {
                        log::warn!("Skipping {} (not a Git repository)", repo_path.display());
                        continue;
                    }

                    let uuid = Repo::read_uuid(&repo_path, cloned_app);
                    if uuid.is_err() {
                        bail!(
                            "Failed to read UUID from repo {}: {}",
                            repo_path.display(),
                            uuid.unwrap_err()
                        );
                    }
                    let uuid = uuid.unwrap();

                    let repo = ServedRepo::new(&repo_path, None);
                    if let Some(_existing) = result.insert(uuid.clone(), repo) {
                        bail!(
                            "Duplicate repo with UUID {} at {}",
                            uuid,
                            repo_path.display()
                        );
                    }
                }
                Err(e) => {
                    bail!("Failed to read directory {}: {}", path.display(), e);
                }
            }
        }

        Ok(result)
    }
}
