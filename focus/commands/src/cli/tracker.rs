use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Display,
    fs::canonicalize,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::{git_helper, sandbox::Sandbox};

fn focus_config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("could not determine config dir")
        .join("focus")
}

pub struct TrackedRepo {
    identifier: Uuid,
    location: PathBuf,
    link_path: PathBuf,
}

impl TrackedRepo {
    pub fn new(identifier: Uuid, location: &Path, link_path: &Path) -> Result<Self> {
        Ok(Self {
            identifier,
            location: location.to_owned(),
            link_path: link_path.to_owned(),
        })
    }

    fn read_uuid(repo_path: &Path, sandbox: &Sandbox) -> Result<Uuid> {
        let uuid = git_helper::read_config(repo_path, "twitter.focus.uuid", sandbox)?;
        let uuid = Uuid::from_str(uuid.as_str()).context("parsing uuid")?;
        log::info!(
            "Read existing UUID {} for repo at path {}",
            uuid.borrow(),
            repo_path.display()
        );
        Ok(uuid)
    }

    fn write_generated_uuid(repo_path: &Path, sandbox: &Sandbox) -> Result<Uuid> {
        let uuid = Uuid::new_v4();
        log::info!(
            "Assigning new UUID {} for repo at path {}",
            uuid.borrow(),
            repo_path.display()
        );
        git_helper::write_config(
            repo_path,
            "twitter.focus.uuid",
            uuid.to_string().as_str(),
            sandbox,
        )
        .context("writing generated uuid")?;
        Ok(uuid)
    }

    pub fn get_or_generate_uuid(repo_path: &Path, sandbox: &Sandbox) -> Result<Uuid> {
        Self::read_uuid(repo_path, sandbox)
            .or_else(|_e| Self::write_generated_uuid(repo_path, sandbox))
    }

    pub fn identifier(&self) -> &Uuid {
        self.identifier.borrow()
    }

    pub fn location(&self) -> &PathBuf {
        self.location.borrow()
    }

    pub fn link_path(&self) -> &PathBuf {
        self.link_path.borrow()
    }
}

impl Display for TrackedRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.location().display(), self.identifier())
    }
}

pub struct Snapshot {
    repos: Vec<TrackedRepo>,
    index_by_identifier: HashMap<Vec<u8>, usize>,
}

impl Snapshot {
    pub fn new(repos: Vec<TrackedRepo>) -> Self {
        let index_by_identifier = repos.iter().enumerate().fold(
            HashMap::<Vec<u8>, usize>::new(),
            |mut index_by_identifier, (index, repo)| {
                assert!(index_by_identifier
                    .insert(repo.identifier.as_bytes().to_vec(), index)
                    .is_none());
                index_by_identifier
            },
        );

        Snapshot {
            repos,
            index_by_identifier,
        }
    }

    pub fn repos(&self) -> &Vec<TrackedRepo> {
        self.repos.borrow()
    }

    pub fn find_repo_by_id(&self, id: &Vec<u8>) -> Option<&TrackedRepo> {
        self.index_by_identifier
            .get(id)
            .and_then(|&index| self.repos.get(index))
    }
}

pub struct Tracker {
    directory: PathBuf,
}

impl Tracker {
    pub fn new(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("creating directory hierarchy '{}'", directory.display()))?;

        Ok(Self {
            directory: directory.to_owned(),
        })
    }

    pub fn ensure_directories_exist(&self) -> Result<()> {
        std::fs::create_dir_all(self.repos_by_uuid_dir()).context("create by-uuid repo dir")?;
        Ok(())
    }

    pub fn ensure_registered(&self, repo_directory: &Path, sandbox: &Sandbox) -> Result<()> {
        let uuid = TrackedRepo::get_or_generate_uuid(repo_directory, sandbox)?;
        let link_path = self.repos_by_uuid_dir().join(uuid.to_string());
        std::os::unix::fs::symlink(repo_directory, link_path.as_path()).with_context(|| {
            format!(
                "creating symlink from {} to {}",
                link_path.display(),
                repo_directory.display()
            )
        })?;
        Ok(())
    }

    // Scan the directory containing repos labeled by UUID.
    pub fn scan(&self) -> Result<Snapshot> {
        let reader = self
            .repos_by_uuid_dir()
            .read_dir()
            .with_context(|| format!("Failed reading directory {}", self.directory.display()))?;
        let mut repos = Vec::<TrackedRepo>::new();

        for entry in reader {
            match entry {
                Ok(entry) => {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_symlink() {
                            match canonicalize(entry.path()) {
                                Ok(canonical_path) => {
                                    let file_name = entry.file_name();
                                    let utf_file_name = file_name.to_str();
                                    if utf_file_name.is_none() {
                                        bail!(
                                            "unable to interpret path {} as utf8",
                                            entry.path().display()
                                        );
                                    }
                                    let uuid = Uuid::parse_str(utf_file_name.unwrap())
                                        .context("parsing file name as a uuid")?;
                                    let repo = TrackedRepo::new(
                                        uuid,
                                        canonical_path.as_path(),
                                        entry.path().borrow(),
                                    )
                                    .context("instantiating tracked repo")?;
                                    repos.push(repo);
                                }

                                Err(e) => {
                                    bail!(
                                        "unable to canonicalize path {}: {}",
                                        entry.path().display(),
                                        e
                                    );
                                }
                            }
                        } else {
                            log::info!("ignoring {} (not a symlink)", entry.path().display());
                        }
                    } else {
                        bail!("unable to determine file type for {:?}", entry.path());
                    }
                }

                Err(e) => {
                    bail!("failed reading directory entry: {}", e);
                }
            }
        }

        Ok(Snapshot::new(repos))
    }

    fn repos_dir(&self) -> PathBuf {
        self.directory.join("repos")
    }

    fn repos_by_uuid_dir(&self) -> PathBuf {
        self.repos_dir().join("by-uuid")
    }
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            directory: focus_config_dir(),
        }
    }
}
