use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Error, Result};
use futures::future::Remote;
use serde_derive::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum RemovalError {
    #[error("not found")]
    NotFound,
    
    #[error("unable to remove mandatory layer")]
    Mandatory,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct Layer {
    name: String,
    description: String,
    mandatory: bool,
    coordinates: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct Topology {
    layers: Vec<Layer>,
}

enum RemoveResult {
    CannotRemoveMandatoryLayer,
    NotFound,
}

impl Topology {
    pub fn validate(&self) -> Result<()> {
        // Find duplicate names
        let mut visited_names = HashMap::<String, usize>::new();
        let mut index: usize = 0;
        for layer in &self.layers {
            let name = &layer.name.to_owned();
            if let Some(existing_index) = visited_names.insert(name.to_owned(), index) {
                bail!(
                    "Layer named '{}' at index {} has the same name as existing layer at index {}",
                    &name,
                    index,
                    existing_index
                );
            } else {
            }

            index += 1;
        }
        Ok(())
    }

    pub fn extend(&mut self, other: &Topology) {
        self.layers.extend(other.layers.clone());
    }


    pub fn remove_named_layer(&mut self, name: &str) -> Result<()> {
        for (ix, l) in self.layers.iter().enumerate() {
            if l.name.eq(&name) {
                if l.mandatory {
                    return Err(Error::new(RemovalError::Mandatory));
                }
                self.layers.remove(ix);
                return Ok(());
            }
        }

        return Err(Error::new(RemovalError::NotFound));
    }

    fn load(path: &Path) -> Result<Topology> {
        Ok(serde_json::from_slice(&ByteStore::load(&path)?)?)
    }

    fn store(path: &Path, t: &Topology) -> Result<()> {
        ByteStore::store(&serde_json::to_vec(&t)?, &path)?;
        Ok(())
    }
}
pub struct ByteStore {}

impl ByteStore {
    fn store(buf: &[u8], output_path: &Path) -> Result<()> {
        use std::io::prelude::*;
        let mut file = File::create(&output_path).context("opening for write")?;
        file.write_all(&buf).context("writing buffer to file")?;
        file.sync_all().context("syncing file")?;
        Ok(())
    }

    fn load(input_path: &Path) -> Result<Vec<u8>> {
        let mut file = File::open(&input_path)?;
        let mut buf = Vec::<u8>::new();
        file.read_to_end(&mut buf)
            .context("reading bytes into buffer")?;
        Ok(buf)
    }
}

pub struct Topologies {
    path: PathBuf,
}

impl Topologies {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use sha2::digest::generic_array::typenum::assert_type;
    use tempfile::tempdir;

    fn layers() -> Vec<Layer> {
        vec![
            Layer {
                name: "baseline/tools_implicit_deps".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                coordinates: vec!["//tools/implicit_deps:thrift-implicit-deps-impl".to_owned()],
            },
            Layer {
                name: "baseline/scrooge_internal".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                coordinates: vec!["//tools/implicit_deps:thrift-implicit-deps-impl".to_owned()],
            },
            Layer {
                name: "baseline/loglens".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                coordinates: vec!["//scrooge-internal/...".to_owned()],
            },
            Layer {
                name: "projects/cdpain".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                coordinates: vec!["//workflows/examples/cdpain/...".to_owned()],
            },
        ]
    }

    fn topology() -> Topology {
        Topology { layers: layers() }
    }

    #[test]
    fn validate() -> Result<()> {
        {
            let topology = topology();
            assert!(topology.validate().is_ok());
        }

        {
            let mut layers = layers();
            layers.push(Layer {
                name: "baseline/loglens".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                coordinates: vec!["it doesn't matter".to_owned()],
            });
            let topology = Topology { layers };
            let e = topology.validate().unwrap_err();
            assert_eq!("Layer named 'baseline/loglens' at index 3 has the same name as existing layer at index 2",e.to_string());
        }

        Ok(())
    }

    #[test]
    fn merge() -> Result<()> {
        let mut t1 = topology();
        let t2 = Topology {
            layers: vec![Layer {
                name: "foo".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                coordinates: vec!["//foo/bar/...".to_owned()],
            }],
        };

        t1.extend(&t2);
        assert_eq!(&t1.layers.last().unwrap(), &t2.layers.last().unwrap());
        Ok(())
    }

    #[test]
    fn remove_named_layer() -> Result<()> {
        let mut topology = topology();
        topology.remove_named_layer("projects/cdpain")?;

        Ok(())
    }
    
    #[test]
    fn remove_named_layer_not_found() -> Result<()> {
        let mut topology = topology();
        let result = topology.remove_named_layer("baseline/boo");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().root_cause().to_string(), RemovalError::NotFound.to_string());

        Ok(())
    }

    #[test]
    fn remove_named_layer_cannot_remove_mandatory_layers() -> Result<()> {
        let mut topology = topology();
        let result = topology.remove_named_layer("baseline/loglens");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().root_cause().to_string(), RemovalError::Mandatory.to_string());

        Ok(())
    }

    #[test]
    fn blob_serialization() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("topo");
        let topo = topology();
        ByteStore::store(&serde_json::to_vec(&topo)?, &path)?;
        let loaded_topo: Topology = serde_json::from_slice(&ByteStore::load(&path)?)?;
        assert_eq!(loaded_topo, topo);
        Ok(())
    }
}
