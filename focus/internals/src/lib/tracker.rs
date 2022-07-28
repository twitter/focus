// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Display,
    fs::canonicalize,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use tracing::{debug, info};
use uuid::Uuid;

use focus_util::{app::App, lock_file::LockFile, paths::focus_config_dir};

use crate::model::repo::Repo;

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
        let repo = Repo::open(repo_path, app)?;
        if let Some(working_tree) = repo.working_tree() {
            match working_tree.read_uuid() {
                Ok(Some(uuid)) => Ok(uuid),
                _ => working_tree.write_generated_uuid(),
            }
        } else {
            bail!("No working tree");
        }
    }

    pub fn identifier(&self) -> &Uuid {
        self.identifier.borrow()
    }

    pub fn location(&self) -> &Path {
        self.location.borrow()
    }

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

    pub fn find_repo_by_id(&self, id: &[u8]) -> Option<&TrackedRepo> {
        self.index_by_identifier
            .get(id)
            .and_then(|&index| self.repos.get(index))
    }
}

#[derive(Debug)]
pub struct Tracker {
    _tempdir: Option<tempfile::TempDir>,
    directory: PathBuf,
}

impl Tracker {
    pub fn new(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("creating directory hierarchy '{}'", directory.display()))?;

        Ok(Self {
            _tempdir: None,
            directory: directory.to_owned(),
        })
    }

    pub fn from_config_dir() -> Result<Self> {
        if cfg!(test) {
            panic!("Should not be trying to construct a real Tracker object during testing, as it reads/writes from the user's config directory");
        }
        Ok(Self {
            _tempdir: None,
            directory: focus_config_dir(),
        })
    }

    pub fn for_testing() -> Result<Self> {
        let tempdir = tempfile::tempdir()?;
        let directory = tempdir.path().to_path_buf();
        std::fs::create_dir_all(&directory)?;
        Ok(Self {
            _tempdir: Some(tempdir),
            directory,
        })
    }

    pub fn ensure_directories_exist(&self) -> Result<()> {
        std::fs::create_dir_all(self.repos_by_uuid_dir()).context("create by-uuid repo dir")?;
        Ok(())
    }

    pub fn ensure_registered(&self, repo_directory: &Path, app: Arc<App>) -> Result<()> {
        let uuid = TrackedRepo::get_or_generate_uuid(repo_directory, app)?;
        let link_path = self.repos_by_uuid_dir().join(uuid.to_string());
        if link_path.is_symlink() {
            if let Ok(path) = std::fs::read_link(&link_path) {
                let canonical_repo_dir = repo_directory
                    .canonicalize()
                    .context("Canonicalizing repo path")?;
                let canonical_link_target = path
                    .canonicalize()
                    .context("Canonicalzing existing symlink path")?;
                if canonical_repo_dir == canonical_link_target {
                    // The symlink already exists.
                    info!(?canonical_repo_dir, "Symlink already exists");
                    return Ok(());
                }
            }
        }

        std::os::unix::fs::symlink(repo_directory, link_path.as_path()).with_context(|| {
            format!(
                "creating symlink from {} to {}",
                link_path.display(),
                repo_directory.display()
            )
        })?;
        Ok(())
    }

    fn repo_registry_lock_path(&self) -> PathBuf {
        self.repos_by_uuid_dir().join("regisry.lock")
    }

    // Repair the registry of tracked repositories by checking that symlinks point to canonicalizable destinations and that the configured UUIDs match the inbound link.
    pub fn repair(&self, app: Arc<App>) -> Result<()> {
        // Hold a repo repair lock.
        let _lock = LockFile::new(&self.repo_registry_lock_path());

        let reader = self
            .repos_by_uuid_dir()
            .read_dir()
            .with_context(|| format!("Failed reading directory {}", self.directory.display()))?;

        for entry in reader {
            match entry {
                Ok(entry) => {
                    let entry_path = entry.path();
                    info!(?entry_path, "Checking repository");

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

                                    let repo = Repo::open(&canonical_path, app.clone())?;
                                    let uuid_from_config =
                                        if let Some(working_tree) = repo.working_tree() {
                                            Ok(working_tree.read_uuid()?)
                                        } else {
                                            Err("No working tree")
                                        };

                                    let mismatched_uuid = match uuid_from_config {
                                        Ok(Some(configured)) => configured != uuid_from_filename,
                                        _ => true,
                                    };

                                    if mismatched_uuid {
                                        debug!(
                                            ?entry_path,
                                            ?uuid_from_filename,
                                            "Removing entry configured UUID differs from indicated UUID",
                                        );
                                        std::fs::remove_file(entry.path())?;
                                    }
                                }

                                Err(e) => {
                                    debug!(?entry_path, ?e, "Removing entry: invalid destination",);
                                    std::fs::remove_file(entry.path())?;
                                }
                            }
                        } else {
                            debug!(?entry_path, "Removing entry (not a symlink)");
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
                        let entry_path = entry.path();
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
                                    debug!(
                                        ?entry_path,
                                        ?e,
                                        "Skipping entry: unable to canonicalize destination",
                                    );
                                }
                            }
                        } else {
                            debug!(?entry_path, "ignoring entry (not a symlink)");
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
