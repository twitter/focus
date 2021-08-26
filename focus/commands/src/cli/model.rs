use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt::Display,
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

    #[serde(default)]
    mandatory: bool,

    coordinates: Vec<String>,
}

impl Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{} ({}) -> {:?}",
            &self.name,
            if self.mandatory { " <mandatory>" } else { "" },
            &self.description,
            &self.coordinates,
        )
    }
}

impl Layer {
    pub fn new(name: &str, description: &str, mandatory: bool, coordinates: &Vec<String>) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            mandatory,
            coordinates: coordinates.to_owned(),
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn coordinates(&self) -> &Vec<String> {
        &self.coordinates
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct LayerSet {
    layers: Vec<Layer>,

    #[serde(skip)]
    content_hash: Option<String>, // Hex representation of a SHA-256 checksum
}

enum RemoveResult {
    CannotRemoveMandatoryLayer,
    NotFound,
}

impl LayerSet {
    pub fn new(layers: Vec<Layer>) -> Self {
        Self {
            layers,
            content_hash: None,
        }
    }

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

    pub fn layers(&self) -> &Vec<Layer> {
        &self.layers
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
        let slice = &std::fs::read(&path).context("opening file for read")?;

        let mut layer_set: LayerSet = serde_json::from_slice(&slice)
            .with_context(|| format!("loading layer set from {}", &path.display()))?;

        // let mut hasher = Sha256::new();
        // hasher.update(&slice);
        // layer_set.content_hash = Some(format!("{:x}", hasher.finalize()));
        layer_set.content_hash = None;

        Ok(layer_set)
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

// Selections are stacks of pointers to layers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct LayerStack {
    selected_layer_names: Vec<String>,
}

impl LayerStack {
    pub fn load(path: &Path) -> Result<LayerStack> {
        Ok(
            serde_json::from_slice(&std::fs::read(&path).context("opening file for read")?)
                .context("loading layer stack")?,
        )
    }

    pub fn store(path: &Path, t: &LayerStack) -> Result<()> {
        std::fs::write(
            &path,
            &serde_json::to_vec(&t).context("opening file for write")?,
        )
        .context("storing layer stack")?;

        Ok(())
    }
}

// RichLayerSet adds indexing to layer sets and
pub struct RichLayerSet {
    underlying: LayerSet,
    index_on_name: RefCell<HashMap<String, usize>>,
}

impl<'a> RichLayerSet {
    pub fn new(underlying: LayerSet) -> Result<Self> {
        let mut instance = Self {
            underlying,
            index_on_name: RefCell::new(HashMap::new()),
        };

        Self::index(&instance.underlying, instance.index_on_name.get_mut())?;

        Ok(instance)
    }

    fn index(layer_set: &LayerSet, index_map: &mut HashMap<String, usize>) -> Result<()> {
        for (index, layer) in layer_set.layers.iter().enumerate() {
            if let Some(existing) = index_map.insert(layer.name.clone(), index) {
                bail!(
                    "Layer {:?} has the same name as layer {:?}",
                    &layer,
                    &layer_set.layers[existing]
                );
            }
        }
        Ok(())
    }

    fn reindex(&self) -> Result<()> {
        let mut new_index = HashMap::<String, usize>::new();
        Self::index(&self.underlying, &mut new_index)?;
        self.index_on_name.replace(new_index);
        Ok(())
    }

    pub fn find_index(&self, name: &str) -> Option<usize> {
        let index_on_name = self.index_on_name.borrow();
        if let Some(ix) = index_on_name.get(name) {
            return Some(*ix);
        }

        None
    }

    pub fn get(&self, name: &str) -> Option<&Layer> {
        if let Some(ix) = self.find_index(&name) {
            let layer: &Layer = &self.underlying.layers[ix];
            return Some(layer);
        }

        None
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.index_on_name.borrow().contains_key(name)
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

    pub fn user_directory(&self) -> PathBuf {
        self.repo_path.join(".focus")
    }

    // The layers the user has chosen
    pub fn selected_layer_stack_path(&self) -> PathBuf {
        self.user_directory().join("user.stack.json")
    }

    pub fn adhoc_layer_path(&self) -> PathBuf {
        self.user_directory().join("adhoc.layers.json")
    }

    // The directory containing project-oriented layers. All .layers.json will be scanned.
    pub fn project_directory(&self) -> PathBuf {
        self.repo_path.join("focus").join("projects")
    }

    pub fn mandatory_layer_path(&self) -> PathBuf {
        self.repo_path.join("focus").join("mandatory.layers.json")
    }

    fn layer_file_filter(entry: &DirEntry) -> bool {
        if entry.path().is_dir() {
            return true;
        }

        let suffix = OsString::from(".layers.json");
        let ostr = entry.path().as_os_str();
        if ostr.len() < suffix.len() {
            return false;
        }

        ostr.as_bytes().ends_with(suffix.as_bytes())
    }

    fn locate_layer_set_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut results = Vec::<PathBuf>::new();
        let walker = WalkDir::new(path)
            .sort_by_file_name()
            .follow_links(true)
            .into_iter();
        log::debug!(
            "scanning project directory {}",
            &self.project_directory().display()
        );

        for entry in walker.filter_entry(|e| Self::layer_file_filter(&e)) {
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

    // Return a layer_set containing all mandatory layers
    pub fn mandatory_layers(&self) -> Result<LayerSet> {
        LayerSet::load(&self.mandatory_layer_path()).context("loading mandatory layer set")
    }

    pub fn adhoc_layers(&self) -> Result<Option<LayerSet>> {
        if !self.adhoc_layer_path().is_file() {
            return Ok(None);
        }

        Ok(Some(
            LayerSet::load(&self.adhoc_layer_path()).context("loading adhoc layer set")?,
        ))
    }

    pub fn store_adhoc_layers(&self, layer_set: &LayerSet) -> Result<()> {
        LayerSet::store(self.adhoc_layer_path().as_path(), layer_set)
            .context("storing ad hoc layer set")
    }

    // Return a layer_set cataloging all available layers
    pub fn available_layers(&self) -> Result<LayerSet> {
        let mut layer = LayerSet {
            layers: vec![],
            content_hash: None,
        };

        let paths = self
            .locate_layer_set_files(&self.project_directory())
            .context("scanning project layer_set files")?;

        for path in &paths {
            layer.extend(
                &LayerSet::load(&path)
                    .with_context(|| format!("loading layer_set from {}", &path.display()))?,
            );
        }

        Ok(layer)
    }

    fn find_named_layers(names: &Vec<String>, set: &RichLayerSet) -> Result<Vec<Layer>> {
        let mut layers = Vec::<Layer>::new();

        for (index, name) in names.iter().enumerate() {
            if let Some(layer) = set.get(name) {
                layers.push(layer.to_owned())
            } else {
                // TODO: Provide an affordance for ignoring missing layers
                return Err(Error::new(LoadError::NotFound).context(format!(
                    "Layer named '{}' (at index {}) is not present",
                    &name, index
                )));
            }
        }

        Ok(layers)
    }

    pub fn user_layers(&self) -> Result<Option<LayerStack>> {
        let path = self.selected_layer_stack_path();
        if !path.exists() {
            return Ok(None);
        }

        Ok(Some(
            LayerStack::load(&path).context("loading user layer stack")?,
        ))
    }

    // Return a layer_set containing the layers a user has selected
    pub fn selected_layers(&self) -> Result<Option<LayerSet>> {
        let layer_stack: LayerStack;
        if let Ok(Some(stack)) = self.user_layers() {
            layer_stack = stack;
        } else {
            return Ok(None);
        }

        let indexed_available_layers = RichLayerSet::new(
            self.available_layers()
                .context("loading available layers")?,
        )?;
        let layers =
            Self::find_named_layers(&layer_stack.selected_layer_names, &indexed_available_layers)
                .context("extracting selected layers from the set of all available layers")?;

        Ok(Some(LayerSet {
            layers,
            content_hash: None,
        }))
    }

    // Return the computed layers, namely the mandatory layers and the selected layers
    fn computed_layers(&self) -> Result<LayerSet> {
        let mut layers = self
            .mandatory_layers()
            .context("loading mandatory layers")?;
        if let Some(adhoc_layers) = self.adhoc_layers().context("loading ad hoc layers")? {
            layers.extend(&adhoc_layers);
        }
        if let Some(selected_layers) = self.selected_layers().context("loading selected layers")? {
            layers.extend(&selected_layers);
        } else {
            log::warn!("No layers are selected!");
        }
        Ok(layers)
    }

    fn store_selected_layers(&self, stack: &LayerStack) -> Result<()> {
        std::fs::create_dir_all(self.user_directory())
            .context("creating the directory to store user layers")?;
        LayerStack::store(&self.selected_layer_stack_path(), &stack)
            .context("storing user layer stack")
    }

    pub fn push_as_selection(&self, names: Vec<String>) -> Result<LayerSet> {
        // TODO: Locking
        let mut user_layers = self
            .user_layers()
            .context("loading user layers")?
            .unwrap_or_default();
        let mut selected = self.selected_layers()?.unwrap_or_default();
        let selected_indexed = RichLayerSet::new(selected.clone())?;
        let available = RichLayerSet::new(self.available_layers()?)?;

        for name in names {
            if selected_indexed.contains_key(&name) {
                // Already have this one
                eprintln!("{}: Skipped (already selected)", &name)
            } else {
                if let Some(layer) = available.get(&name) {
                    // let name_clone = name.to_owned().to_owned();
                    user_layers.selected_layer_names.push(name.clone());
                    selected.layers.push(layer.clone());
                } else {
                    eprintln!("{}: Not found", &name);
                    bail!("One of the requested layers was not found");
                }
            }
        }

        self.store_selected_layers(&user_layers)
            .context("storing the modified user layer stack")?;

        Ok(selected)
    }

    pub fn pop(&self, count: usize) -> Result<LayerSet> {
        // TODO: Locking
        let mut user_layers = self
            .user_layers()
            .context("loading user layers")?
            .unwrap_or_default();
        let mut selected = self.selected_layers()?.unwrap_or_default();

        for _ in 0..count {
            user_layers.selected_layer_names.pop();
            selected.layers.pop();
        }

        self.store_selected_layers(&user_layers)
            .context("storing the modified user layer stack")?;

        Ok(selected)
    }

    pub fn remove(&self, names: Vec<String>) -> Result<LayerSet> {
        // TODO: Locking
        let user_layers = self
            .user_layers()
            .context("loading user layers")?
            .unwrap_or_default();

        let mut name_set = HashSet::<String>::new();
        // let names_refs: Vec<&String> = names.iter().map(|name| name.to_owned()).collect();
        name_set.extend(names);
        let mut removals: usize = 0;
        let retained: Vec<String> = user_layers
            .selected_layer_names
            .iter()
            .filter_map(|name| {
                if name_set.contains(name) {
                    removals += 1;
                    None
                } else {
                    Some(name.clone())
                }
            })
            .collect::<_>();

        if removals == 0 {
            eprintln!("No layers matched; nothing removed!");
        }

        let new_layers = LayerStack {
            selected_layer_names: retained,
        };

        self.store_selected_layers(&new_layers)
            .context("storing the modified user layer stack")?;

        Ok(self
            .selected_layers()
            .context("loading selected layers")?
            .unwrap_or_default())
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
        LayerSet {
            layers: layers(),
            content_hash: None,
        }
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
            let layer_set = LayerSet {
                layers,
                content_hash: None,
            };
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
            content_hash: None,
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
            content_hash: None,
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
        let builtins_path = project_dir.join("builtins.layers.json");
        LayerSet::store(&builtins_path, &builtins_layer).context("storing builtins_layer")?;

        Ok((dir, t))
    }

    #[test]
    fn available_layers() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        let project_dir = t.project_directory();

        let my_project_path = project_dir.join("my_project.layers.json");
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
        let t = LayerSet {
            layers: ls,
            content_hash: None,
        };

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
