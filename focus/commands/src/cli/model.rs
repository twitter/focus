use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::Read,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Error, Result};
use serde_derive::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

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
        Ok(
            serde_json::from_slice(&std::fs::read(&path).context("opening file for read")?)
                .context("storing topology")?,
        )
    }

    fn store(path: &Path, t: &Topology) -> Result<()> {
        std::fs::write(
            &path,
            &serde_json::to_vec(&t).context("opening file for write")?,
        )
        .context("storing topology")?;

        Ok(())
    }
}

pub struct Topologies {
    repo_path: PathBuf,
}

impl Topologies {
    pub fn new(repo_path: &Path) -> Self {
        Self {
            repo_path: repo_path.to_owned(),
        }
    }

    // The layers the user has chosen
    pub fn user_topology_path(&self) -> PathBuf {
        self.repo_path.join(".focus").join("user.topo.json")
    }

    // The directory containing project-oriented layers. All .topo.json will be scanned.
    pub fn project_directory(&self) -> PathBuf {
        self.repo_path.join("focus")
    }

    fn topo_json_filter(entry: &DirEntry) -> bool {
        if entry.path().is_dir() {
            return true;
        }

        let suffix = OsString::from(".topo.json");
        let ostr = entry.path().as_os_str();
        if ostr.len() < suffix.len() {
            return false;
        }

        ostr.as_bytes().ends_with(suffix.as_bytes())
    }

    fn scan_projects(&self) -> Result<Vec<PathBuf>> {
        let mut results = Vec::<PathBuf>::new();
        let walker = WalkDir::new(self.project_directory())
            .sort_by_file_name()
            .follow_links(true)
            .into_iter();
        log::debug!("scanning project directory {}", &self.project_directory().display());

        for entry in walker.filter_entry(|e| Self::topo_json_filter(&e)) {
            
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    if !path.is_file() {
                        // Ignore non-files -- walkdir includes them.
                        continue;
                    }
                    log::debug!("Processing project file {}", &path.display());
                    results.push(path.to_owned());
                }
                Err(e) => {
                    log::warn!("Encountered error: {}", e);
                }
            }
        }

        return Ok(results);
    }

    // Return a catalog of project topologies (excludes mandatory topologies)
    pub fn catalog(&self) -> Result<Topology> {
        let mut catalog_topo = Topology { layers: vec![] };

        let paths = self
            .scan_projects()
            .context("scanning project topology files")?;

        for path in &paths {
            let path = path.as_path();
            let topo = Topology::load(&path)
                .with_context(|| format!("loading topology from {}", &path.display()))?;
            catalog_topo.extend(&topo);
        }

        Ok(catalog_topo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use tempfile::{tempdir, TempDir};

    fn init_logging() {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

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
        init_logging();

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
        init_logging();

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
        init_logging();

        let mut topology = topology();
        topology.remove_named_layer("projects/cdpain")?;

        Ok(())
    }

    #[test]
    fn remove_named_layer_not_found() -> Result<()> {
        init_logging();

        let mut topology = topology();
        let result = topology.remove_named_layer("baseline/boo");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().root_cause().to_string(),
            RemovalError::NotFound.to_string()
        );

        Ok(())
    }

    #[test]
    fn remove_named_layer_cannot_remove_mandatory_layers() -> Result<()> {
        init_logging();

        let mut topology = topology();
        let result = topology.remove_named_layer("baseline/loglens");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().root_cause().to_string(),
            RemovalError::Mandatory.to_string()
        );

        Ok(())
    }

    fn project_fixture(name: &str) -> Topology {
        Topology {
            layers: vec![Layer {
                name: name.to_owned(),
                description: format!("Fixture topology {}", name),
                mandatory: false,
                coordinates: vec![format!("//{}/...", name)],
            }],
        }
    }

    fn repo_fixture() -> Result<(TempDir, Topologies)> {
        let dir = tempdir().context("making a temporary directory")?;
        let path = dir.path().join("test_repo");
        let t = Topologies::new(&path);
        let project_dir = t.project_directory();
        std::fs::create_dir_all(&project_dir).context("creating project dir")?;

        let random_file_path = project_dir.join("whatever.json");
        std::fs::write(&random_file_path, b"{}").context("writing random file")?;

        let builtins_topo = topology();
        let builtins_path = project_dir.join("builtins.topo.json");
        Topology::store(&builtins_path, &builtins_topo).context("storing builtins_topo")?;

        Ok((dir, t))
    }

    #[test]
    fn catalog() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        let project_dir = t.project_directory();

        let my_project_path = project_dir.join("my_project.topo.json");
        let my_project = project_fixture("my_project");
        Topology::store(&my_project_path, &my_project).context("storing my_project")?;
        log::info!("stored to {}", &my_project_path.display());

        let cat = t.catalog().context("reading catalog")?;
        assert_eq!(cat.layers.len(), 5);

        Ok(())
    }
}
