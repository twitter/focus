use std::{
    collections::HashMap,
    hash::Hash,
    path::{Path, PathBuf},
};

use anyhow::{Context, Error, Result};
use serde_json::map::Iter;
use uuid::Uuid;

fn focus_config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("could not determine config dir")
        .join("focus")
}

pub struct RegisteredRepo {
    identifier: Uuid,
    location: PathBuf,
    link_path: PathBuf,
}

impl RegisteredRepo {
    pub fn new(identifier: Uuid, location: &Path, link_path: &Path) -> Result<Self> {
        Ok(Self {
            identifier,
            location: location.to_owned(),
            link_path: link_path.to_owned(),
        })
    }
}

pub struct Snapshot {
    repos: Vec<RegisteredRepo>,
    index_by_identifier: HashMap<Vec<u8>, usize>,
}

impl Snapshot {
    pub fn new(repos: Vec<RegisteredRepo>) -> Self {
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
}

// struct IndexedModelRepository<Key, Item> {
//     items: Vec<Item>,
//     index: HashMap<Key, usize>,
// }

// impl<Key, Item> IndexedModelRepository<Key, Item> {
//     pub fn from_items<I, Input>(items: I) -> Self
//     where
//         Key: Eq + Hash,
//         I: Iterator<Item = Input>,
//     {
//         let items = Vec::<Item>::new();
//         let index = HashMap::<Key, usize>::new();
//         for i in items {
//             // let (key, item) = i;
//             // let index = items.len();
//             // items.push(i);
//             // index.insert(k, items.len());
//         }

//         Self { items, index }
//     }
// }

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

    pub fn scan(&self) -> Result<Snapshot> {
        let reader = self
            .repos_by_uuid_dir()
            .read_dir()
            .with_context(|| format!("Failed reading directory {}", self.directory.display()))?;

        // for entry in reader {
        //     match entry {
        //         Ok(entry) => {

        //         }, 
        //         Err(e) => {
        //             return Err(Error::new(e).with_context(|| format!("Failed reading directory {}", self.directory.display()))
        //         },
        //     }
        // }

        // Ok()
        todo!("implement");
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
