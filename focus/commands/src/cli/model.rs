use std::{
    collections::HashMap,
    ffi::OsString,
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

#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("not found")]
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct Layer {
    name: String,
    description: String,
    mandatory: bool,
    coordinates: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct LayerSet {
    layers: Vec<Layer>,
}

enum RemoveResult {
    CannotRemoveMandatoryLayer,
    NotFound,
}

impl LayerSet {
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

    pub fn extend(&mut self, other: &LayerSet) {
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

    pub fn optional_layers(&self) -> Result<Vec<&Layer>> {
        Ok(self
            .layers
            .iter()
            .filter_map(|l| if !l.mandatory { Some(l) } else { None })
            .collect())
    }

    fn load(path: &Path) -> Result<LayerSet> {
        Ok(
            serde_json::from_slice(&std::fs::read(&path).context("opening file for read")?)
                .context("storing layer_set")?,
        )
    }

    fn store(path: &Path, t: &LayerSet) -> Result<()> {
        std::fs::write(
            &path,
            &serde_json::to_vec(&t).context("opening file for write")?,
        )
        .context("storing layer_set")?;

        Ok(())
    }
}

pub struct LayerSets {
    repo_path: PathBuf,
}

impl LayerSets {
    pub fn new(repo_path: &Path) -> Self {
        Self {
            repo_path: repo_path.to_owned(),
        }
    }

    pub fn selected_directory(&self) -> PathBuf {
        self.repo_path.join(".focus")
    }

    // The layers the user has chosen
    pub fn selected_layer_set_path(&self) -> PathBuf {
        self.selected_directory().join("user.layer.json")
    }

    // The directory containing project-oriented layers. All .layer.json will be scanned.
    pub fn project_directory(&self) -> PathBuf {
        self.repo_path.join("focus")
    }

    fn layer_json_filter(entry: &DirEntry) -> bool {
        if entry.path().is_dir() {
            return true;
        }

        let suffix = OsString::from(".layer.json");
        let ostr = entry.path().as_os_str();
        if ostr.len() < suffix.len() {
            return false;
        }

        ostr.as_bytes().ends_with(suffix.as_bytes())
    }

    fn locate_layer_set_files(&self) -> Result<Vec<PathBuf>> {
        let mut results = Vec::<PathBuf>::new();
        let walker = WalkDir::new(self.project_directory())
            .sort_by_file_name()
            .follow_links(true)
            .into_iter();
        log::debug!(
            "scanning project directory {}",
            &self.project_directory().display()
        );

        for entry in walker.filter_entry(|e| Self::layer_json_filter(&e)) {
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    if path.is_file() {
                        results.push(path.to_owned());
                    }
                }
                Err(e) => {
                    log::warn!("Encountered error: {}", e);
                }
            }
        }

        return Ok(results);
    }

    // Return a layer_set cataloging all available layers
    pub fn available_layers(&self) -> Result<LayerSet> {
        let mut layer = LayerSet { layers: vec![] };

        let paths = self
            .locate_layer_set_files()
            .context("scanning project layer_set files")?;

        for path in &paths {
            layer.extend(
                &LayerSet::load(&path)
                    .with_context(|| format!("loading layer_set from {}", &path.display()))?,
            );
        }

        Ok(layer)
    }

    // Return a layer_set containing the layers a user has selected
    fn selected_layers(&self) -> Result<Option<LayerSet>> {
        let path = self.selected_layer_set_path();
        if !path.exists() {
            return Ok(None);
        }

        LayerSet::load(&path)
            .context("loading the user layer_set")
            .map(|t| Some(t))
    }

    fn store_selected_layers(&self, t: &LayerSet) -> Result<()> {
        std::fs::create_dir_all(self.selected_directory())
            .context("creating the directory to store user layers")?;
        LayerSet::store(&self.selected_layer_set_path(), &t).context("storing user layers")
    }

    fn add_to_selection(&self) -> Result<LayerSet> {
        let selection = self
            .selected_layers()
            .unwrap_or_default()
            .unwrap_or_default();
        todo!("impl");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::Once;
    use tempfile::{tempdir, TempDir};

    static INIT_LOGGING_ONCE: Once = Once::new();

    fn init_logging() {
        INIT_LOGGING_ONCE.call_once(|| {
            let _ =
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                    .init();
        });
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

    fn layer_set() -> LayerSet {
        LayerSet { layers: layers() }
    }

    #[test]
    fn validate() -> Result<()> {
        init_logging();

        {
            let layer_set = layer_set();
            assert!(layer_set.validate().is_ok());
        }

        {
            let mut layers = layers();
            layers.push(Layer {
                name: "baseline/loglens".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                coordinates: vec!["it doesn't matter".to_owned()],
            });
            let layer_set = LayerSet { layers };
            let e = layer_set.validate().unwrap_err();
            assert_eq!("Layer named 'baseline/loglens' at index 4 has the same name as existing layer at index 2",e.to_string());
        }

        Ok(())
    }

    #[test]
    fn merge() -> Result<()> {
        init_logging();

        let mut t1 = layer_set();
        let t2 = LayerSet {
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

        let mut layer_set = layer_set();
        layer_set.remove_named_layer("projects/cdpain")?;

        Ok(())
    }

    #[test]
    fn remove_named_layer_not_found() -> Result<()> {
        init_logging();

        let mut layer_set = layer_set();
        let result = layer_set.remove_named_layer("baseline/boo");
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

        let mut layer_set = layer_set();
        let result = layer_set.remove_named_layer("baseline/loglens");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().root_cause().to_string(),
            RemovalError::Mandatory.to_string()
        );

        Ok(())
    }

    fn project_fixture(name: &str) -> LayerSet {
        LayerSet {
            layers: vec![Layer {
                name: name.to_owned(),
                description: format!("Fixture layer_set {}", name),
                mandatory: false,
                coordinates: vec![format!("//{}/...", name)],
            }],
        }
    }

    fn repo_fixture() -> Result<(TempDir, LayerSets)> {
        let dir = tempdir().context("making a temporary directory")?;
        let path = dir.path().join("test_repo");
        let t = LayerSets::new(&path);
        let project_dir = t.project_directory();
        std::fs::create_dir_all(&project_dir).context("creating project dir")?;

        let random_file_path = project_dir.join("whatever.json");
        std::fs::write(&random_file_path, b"{}").context("writing random file")?;

        let builtins_layer = layer_set();
        let builtins_path = project_dir.join("builtins.layer.json");
        LayerSet::store(&builtins_path, &builtins_layer).context("storing builtins_layer")?;

        Ok((dir, t))
    }

    #[test]
    fn available_layers() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        let project_dir = t.project_directory();

        let my_project_path = project_dir.join("my_project.layer.json");
        let my_project = project_fixture("my_project");
        LayerSet::store(&my_project_path, &my_project).context("storing my_project")?;

        let cat = t.available_layers().context("reading available_layers")?;
        assert_eq!(cat.layers.len(), 5);

        Ok(())
    }

    #[test]
    fn optional_layers() -> Result<()> {
        init_logging();
        let ls = vec![
            Layer {
                name: "a".to_owned(),
                description: "".to_owned(),
                coordinates: vec!["//a/...".to_owned()],
                mandatory: true,
            },
            Layer {
                name: "b".to_owned(),
                description: "".to_owned(),
                coordinates: vec!["//b/...ı".to_owned()],
                mandatory: false,
            },
        ];
        let t = LayerSet { layers: ls };

        let layers = t.optional_layers()?;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers.last().unwrap().name, "b");

        Ok(())
    }

    fn selected_layers() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        assert!(t.selected_layers().unwrap().is_none());

        Ok(())
    }
}
