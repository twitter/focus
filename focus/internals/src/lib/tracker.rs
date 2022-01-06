use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Display,
    fs::canonicalize,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::{app::App, repository::Repo, util::paths::focus_config_dir};

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

    pub fn get_or_generate_uuid(repo_path: &Path, app: Arc<App>) -> Result<Uuid> {
        let cloned_app = app.clone();
        Repo::read_uuid(repo_path, app)
            .or_else(|_e| Repo::write_generated_uuid(repo_path, cloned_app))
    }

    pub fn identifier(&self) -> &Uuid {
        self.identifier.borrow()
    }

    pub fn location(&self) -> &Path {
        self.location.borrow()
    }

    #[allow(dead_code)]
    pub fn link_path(&self) -> &Path {
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

    pub fn repos(&self) -> &[TrackedRepo] {
        &self.repos
    }

    #[allow(dead_code)]
    pub fn find_repo_by_id(&self, id: &[u8]) -> Option<&TrackedRepo> {
        self.index_by_identifier
            .get(id)
            .and_then(|&index| self.repos.get(index))
    }
}

pub struct Tracker {
    directory: PathBuf,
}

impl Tracker {
    #[allow(unused)]
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

    pub fn ensure_registered(&self, repo_directory: &Path, app: Arc<App>) -> Result<()> {
        let uuid = TrackedRepo::get_or_generate_uuid(repo_directory, app)?;
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

    // Repair the registry of tracked repositories by checking that symlinks point to canonicalizable destinations and that the configured UUIDs match the inbound link.
    pub fn repair(&self, app: Arc<App>) -> Result<()> {
        let reader = self
            .repos_by_uuid_dir()
            .read_dir()
            .with_context(|| format!("Failed reading directory {}", self.directory.display()))?;

        for entry in reader {
            match entry {
                Ok(entry) => {
                    log::info!("Checking {}", entry.path().display());

                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_symlink() {
                            match canonicalize(entry.path()) {
                                Ok(canonical_path) => {
                                    let file_name = entry.file_name();
                                    let utf_file_name = file_name.to_str();
                                    if utf_file_name.is_none() {
                                        bail!(
                                            "Unsable to interpret path {} as UTF-8",
                                            entry.path().display()
                                        );
                                    }
                                    let uuid_from_filename =
                                        Uuid::parse_str(utf_file_name.unwrap())
                                            .context("parsing file name as a uuid")?;

                                    let uuid_from_config =
                                        Repo::read_uuid(&canonical_path, app.clone());

                                    let mismatched_uuid = match uuid_from_config {
                                        Ok(configured) => configured != uuid_from_filename,
                                        Err(_) => true,
                                    };

                                    if mismatched_uuid {
                                        log::warn!(
                                            "Removing {}: configured UUID differs from indicated UUID ({})",
                                            entry.path().display(),
                                            uuid_from_filename,
                                        );
                                        std::fs::remove_file(entry.path())?;
                                    }
                                }

                                Err(e) => {
                                    log::warn!(
                                        "Removing {}: invalid destination: {}",
                                        entry.path().display(),
                                        e
                                    );
                                    std::fs::remove_file(entry.path())?;
                                }
                            }
                        } else {
                            log::warn!("Removing {} (not a symlink)", entry.path().display());
                            std::fs::remove_file(entry.path())?;
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
                                    log::warn!(
                                        "Skipping {}: unable to canonicalize destination {}",
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
