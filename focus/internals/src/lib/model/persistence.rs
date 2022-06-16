use std::{
    collections::HashMap,
    ffi::OsString,
    fs::File,
    io::{BufReader, BufWriter},
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, debug_span};

/// Load value from the specified path, decoding it from a JSON representation.
pub fn load_model<T>(path: impl AsRef<Path>) -> Result<T>
where
    T: Default + DeserializeOwned,
{
    let path = path.as_ref();
    let span = debug_span!("Loading model");
    let _guard = span.enter();
    debug!(?path);
    if let Ok(file) = File::open(path) {
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    } else {
        Ok(Default::default())
    }
}

/// Store value to the specified path, encoding it as a JSON representation.
pub fn store_model<T>(path: impl AsRef<Path>, value: &T) -> Result<()>
where
    T: Serialize,
{
    let path = path.as_ref();
    let span = debug_span!("Storing model");
    let _guard = span.enter();
    debug!(?path);
    let file = File::create(path).with_context(|| {
        format!(
            "Creating or truncating file {} to store model",
            path.display()
        )
    })?;
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, value)?)
}

/// A collection for working with FileBackedModels serialized to a single directory, indexed by name.
pub struct FileBackedCollection<T> {
    directory: PathBuf,
    extension: OsString,
    pub underlying: HashMap<String, T>,
}

impl<T: Default + DeserializeOwned + Serialize> FileBackedCollection<T> {
    /// Constructs a collection, loading all entities from the directory. The `extension` here is somewhat special in that it is treated as a suffix and can therefore contain period characters, allowing for files to have extensions such `foo.json`
    pub fn new(directory: impl AsRef<Path>, extension: OsString) -> Result<Self> {
        let directory = directory.as_ref().to_owned();
        let mut instance = Self {
            directory,
            extension,
            underlying: Default::default(),
        };

        instance.revert()?;

        Ok(instance)
    }

    /// Load entitities from JSON serialized files from the directory whose file names match the given extension.
    pub fn load(&self) -> Result<HashMap<String, T>>
    where
        T: Default + DeserializeOwned,
    {
        let directory = self.directory.as_path();

        if !directory.is_dir() {
            bail!("{} is not a directory", directory.display());
        }

        debug!(desired_extension = ?self.extension);
        let file_suffix = {
            let mut temp = OsString::from(".");
            temp.push(&self.extension);
            temp
        };
        let file_suffix_bytes = file_suffix.as_bytes();

        let dir = std::fs::read_dir(directory)
            .with_context(|| format!("Could not read directory {}", directory.display()))?;
        let mut underlying = HashMap::<String, T>::new();
        for entry in dir {
            let entry = entry?;
            let path = entry.path();
            let file_name_bytes = path.file_name().expect("No file name").as_bytes();
            if !file_name_bytes.ends_with(file_suffix_bytes) {
                debug!(?path, ?file_suffix, "Skipped");
                continue;
            }

            let instance = load_model(&path)
                .with_context(|| format!("deserializing object from {}", path.display()))?;

            let name = file_name_bytes
                .strip_suffix(file_suffix_bytes)
                .expect("Failed to strip file suffix");
            let name = String::from_utf8(name.to_vec())?;
            debug!(?path, ?name, "Successfully read entry");
            underlying.insert(name, instance);
        }

        Ok(underlying)
    }

    #[allow(dead_code)]
    fn make_path(&self, name: &str) -> PathBuf {
        self.directory.join(name).with_extension(&self.extension)
    }

    /// Add an entity to the collection with the given name. In addition to caching the entity in `underlying`, this serializes the representation to disk immediately.
    #[allow(dead_code)]
    pub fn insert(&mut self, name: &str, entity: &T) -> Result<()>
    where
        T: Clone + Serialize,
    {
        let path = self.make_path(name);
        debug!(?path, %name, "Insert");
        self.underlying.insert(name.to_owned(), entity.clone());
        store_model(&path.as_path(), entity)
    }

    /// Remove an entity by name from the `underlying` cache and erase it from disk.
    #[allow(dead_code)]
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let path = self.make_path(name);
        self.underlying.remove(name);
        if path.is_file() {
            debug!(?path, %name, "Deleted persisted entity");
            std::fs::remove_file(path.as_path())
                .with_context(|| format!("Removing file {}", path.display()))?;
        }

        Ok(())
    }

    /// Replace the contents of the cache by loading all entities in the directory from disk.
    #[allow(dead_code)]
    pub fn revert(&mut self) -> Result<()>
    where
        T: Default + DeserializeOwned,
    {
        let updated = self.load().with_context(|| {
            format!(
                "reading entities from directory {}",
                self.directory.display()
            )
        })?;
        self.underlying = updated;
        Ok(())
    }

    /// Save all cached entities to disk.
    #[allow(dead_code)]
    pub fn save(&self) -> Result<()>
    where
        T: Default + DeserializeOwned,
    {
        for (name, entity) in self.underlying.iter() {
            let path = self.make_path(name);
            store_model(&path.as_path(), entity)
                .with_context(|| format!("Storing entity to {}", path.display()))?
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use anyhow::{Ok, Result};
    use serde::{Deserialize, Serialize};

    use super::FileBackedCollection;

    #[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Person {
        name: String,
    }

    fn make_collection(directory: impl AsRef<Path>) -> Result<FileBackedCollection<Person>> {
        FileBackedCollection::<Person>::new(directory, OsString::from("person.json"))
    }

    #[test]
    fn file_backed_collection() -> Result<()> {
        focus_testing::init_logging();

        let dir = tempfile::tempdir()?;
        let mut collection = make_collection(&dir.path())?;
        let mut alternate_collection = make_collection(&dir.path())?;

        let name = "jeff";
        collection.insert(
            name,
            &Person {
                name: String::from("Jeff Lebowski"),
            },
        )?;
        assert!(collection.underlying.contains_key(name));
        assert!(dir.path().join("jeff.person.json").is_file());

        assert!(!alternate_collection.underlying.contains_key(name));
        alternate_collection.revert()?;
        assert!(alternate_collection.underlying.contains_key(name));

        // Remove from first collection
        collection.remove(name)?;
        assert!(!collection.underlying.contains_key(name));
        assert!(!dir.path().join("jeff.person.json").is_file());
        alternate_collection.revert()?;
        assert!(!alternate_collection.underlying.contains_key(name));

        Ok(())
    }

    #[test]
    fn file_backed_collection_subsequent_inserts_overwrite() -> Result<()> {
        focus_testing::init_logging();
        let dir = tempfile::tempdir()?;
        let name = "lebowski";
        {
            let mut collection = make_collection(&dir.path())?;

            collection.insert(
                name,
                &Person {
                    name: String::from("Jeff Lebowski"),
                },
            )?;
            {
                let (actual_name, entity) = collection.underlying.iter().next().unwrap();
                assert_eq!(actual_name, name);
                assert_eq!(entity.name, "Jeff Lebowski");
            }

            collection.insert(
                name,
                &Person {
                    name: String::from("Maude Lebowski"),
                },
            )?;
            {
                let (actual_name, entity) = collection.underlying.iter().next().unwrap();
                assert_eq!(actual_name, name);
                assert_eq!(entity.name, "Maude Lebowski");
            }
        }

        {
            let collection = make_collection(&dir.path())?;
            let (actual_name, entity) = collection.underlying.iter().next().unwrap();
            assert_eq!(actual_name, name);
            assert_eq!(entity.name, "Maude Lebowski");
        }

        Ok(())
    }

    #[test]
    fn file_backed_collection_save() -> Result<()> {
        focus_testing::init_logging();
        let dir = tempfile::tempdir()?;
        {
            let mut collection = make_collection(&dir.path())?;
            {
                collection.underlying.insert(
                    String::from("foo"),
                    Person {
                        name: String::from("foo"),
                    },
                );
                collection.underlying.insert(
                    String::from("bar"),
                    Person {
                        name: String::from("bar"),
                    },
                );
            }

            collection.save()?;
        }

        {
            let collection = make_collection(&dir.path())?;
            assert!(collection.underlying.contains_key("foo"));
            assert!(collection.underlying.contains_key("bar"));
        }

        Ok(())
    }
}
