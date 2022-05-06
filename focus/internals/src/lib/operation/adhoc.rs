use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};

use crate::model::project::{Project, ProjectSet, ProjectSets};
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
        let sets = ProjectSets::new(&self.repo_path);
        let adhoc_layers = sets.adhoc_projects();
        if let Err(e) = adhoc_layers {
            bail!("Loading the ad-hoc project set failed: {}", e);
        };

        let targets = match sets.adhoc_projects().context("loading selected layers")? {
            Some(adhoc) => extract_coordinates(&adhoc),
            None => Default::default(),
        };

        let mut mutated_coordinates = targets.clone();
        visitor_fn(&mut mutated_coordinates)
            .context("Visitor function failed while mutating targets")?;

        if mutated_coordinates != targets {
            let project = Project::new("adhoc", "Ad-hoc target stack", false, mutated_coordinates);
            let updated_set = ProjectSet::new(vec![project]);
            info!("Saving ad-hoc target stack");
            sets.storae_adhoc_project_set(&updated_set)
                .context("Failed storing the ad-hoc target stack project set")?;
            Ok(true)
        } else {
            info!("Skipped saving unchanged ad-hoc target stack",);
            Ok(false)
        }
    }
}

fn extract_coordinates(set: &ProjectSet) -> Vec<String> {
    let mut results = Vec::<String>::new();
    for project in set.projects() {
        for target in project.targets() {
            results.push(target.into());
        }
    }
    results
}

pub fn list(repo: PathBuf) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|targets| {
        if targets.is_empty() {
            eprintln!("The ad-hoc target stack is empty!");
        } else {
            for (index, target) in targets.iter().enumerate() {
                println!("{}: {}", index, target);
            }
        }

        Ok(())
    })
}

pub fn push(repo: PathBuf, names: Vec<String>) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|targets| {
        let mut set = HashSet::<String>::with_capacity(targets.len());
        set.extend(targets.clone());

        for name in &names {
            if set.contains(name) {
                warn!(
                    ?name,
                    "Skipping project since it is already present in the stack",
                )
            } else {
                targets.push(name.to_owned());
            }
        }

        Ok(())
    })
}

pub fn pop(repo: PathBuf, count: usize) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|targets| {
        for i in 0..count {
            if targets.pop().is_none() {
                warn!("There were only {} targets to pop off the stack", i);
                break;
            }
        }

        Ok(())
    })
}

pub fn remove(repo: PathBuf, names: Vec<String>) -> Result<bool> {
    Adhoc::new(repo)?.with_mut_coordinates(|targets| {
        let mut coordinate_index: HashMap<String, usize> = HashMap::new();
        for (index, target) in targets.iter().enumerate() {
            coordinate_index.insert(target.to_owned(), index);
        }

        // names.map { coordinate_index.get(k)}
        for name in &names {
            if let Some(index) = coordinate_index.get(name) {
                targets.remove(*index);
            } else {
                warn!(?name, "Skipped target since it was missing from the stack",);
            }
        }
        Ok(())
    })
}
