use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};

use crate::model::layering::{Layer, LayerSet, LayerSets};
use tracing::{info, warn};

struct Adhoc {
    repo_path: PathBuf,
}

impl Adhoc {
    fn new(repo_path: PathBuf) -> Result<Self> {
        Ok(Self { repo_path })
    }

    pub fn with_mut_coordinates<F>(&self, visitor_fn: F) -> Result<bool>
    where
        F: FnOnce(&mut Vec<String>) -> Result<()>,
    {
        let sets = LayerSets::new(&self.repo_path);
        let adhoc_layers = sets.adhoc_layers();
        if let Err(e) = adhoc_layers {
            bail!("Loading the ad-hoc layer set failed: {}", e);
        };

        let coordinates = match sets.adhoc_layers().context("loading selected layers")? {
            Some(adhoc) => extract_coordinates(&adhoc),
            None => Default::default(),
        };

        let mut mutated_coordinates = coordinates.clone();
        visitor_fn(&mut mutated_coordinates)
            .context("Visitor function failed while mutating coordinates")?;

        if mutated_coordinates != coordinates {
            let layer = Layer::new(
                "adhoc",
                "Ad-hoc coordinate stack",
                false,
                mutated_coordinates,
            );
            let updated_set = LayerSet::new(vec![layer]);
            info!("Saving ad-hoc coordinate stack");
            sets.store_adhoc_layers(&updated_set)
                .context("Failed storing the ad-hoc coordinate stack layer set")?;
            Ok(true)
        } else {
            info!("Skipped saving unchanged ad-hoc coordinate stack",);
            Ok(false)
        }
    }
}

fn extract_coordinates(set: &LayerSet) -> Vec<String> {
    let mut results = Vec::<String>::new();
    for layer in set.layers() {
        for coordinate in layer.coordinates() {
            results.push(coordinate.into());
        }
    }
    results
}

pub fn list(repo: PathBuf) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|coordinates| {
        if coordinates.is_empty() {
            eprintln!("The ad-hoc coordinate stack is empty!");
        } else {
            for (index, coordinate) in coordinates.iter().enumerate() {
                println!("{}: {}", index, coordinate);
            }
        }

        Ok(())
    })
}

pub fn push(repo: PathBuf, names: Vec<String>) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|coordinates| {
        let mut set = HashSet::<String>::with_capacity(coordinates.len());
        set.extend(coordinates.clone());

        for name in &names {
            if set.contains(name) {
                warn!(
                    ?name,
                    "Skipping layer since it is already present in the stack",
                )
            } else {
                coordinates.push(name.to_owned());
            }
        }

        Ok(())
    })
}

pub fn pop(repo: PathBuf, count: usize) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|coordinates| {
        for i in 0..count {
            if coordinates.pop().is_none() {
                warn!("There were only {} coordinates to pop off the stack", i);
                break;
            }
        }

        Ok(())
    })
}

pub fn remove(repo: PathBuf, names: Vec<String>) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|coordinates| {
        let mut coordinate_index: HashMap<String, usize> = HashMap::new();
        for (index, coordinate) in coordinates.iter().enumerate() {
            coordinate_index.insert(coordinate.to_owned(), index);
        }

        // names.map { coordinate_index.get(k)}
        for name in &names {
            if let Some(index) = coordinate_index.get(name) {
                coordinates.remove(*index);
            } else {
                warn!(
                    ?name,
                    "Skipped coordinate since it was missing from the stack",
                );
            }
        }
        Ok(())
    })
}
