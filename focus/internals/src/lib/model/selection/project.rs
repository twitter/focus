use anyhow::{Context, Result};
use std::{collections::BTreeSet, fmt::Display};

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Hash)]
pub struct Project {
    pub name: String,

    pub description: String,

    #[serde(default)]
    pub mandatory: bool,

    pub targets: BTreeSet<String>,
}

impl Ord for Project {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl Display for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{} ({}) -> {:?}",
            &self.name,
            if self.mandatory { " <mandatory>" } else { "" },
            &self.description,
            &self.targets,
        )
    }
}

impl TryFrom<&Project> for TargetSet {
    type Error = anyhow::Error;

    fn try_from(value: &Project) -> Result<Self, Self::Error> {
        let mut target_set = TargetSet::new();

        for target_str in value.targets.iter() {
            let target = Target::try_from(target_str.as_str())?;
            target_set.insert(target);
        }

        Ok(target_set)
    }
}

/// ProjectCatalog allows for retrieval of projects (optional or mandatory) defined in a repository.
pub struct ProjectCatalog {
    pub optional_projects: Projects,
    pub mandatory_projects: Projects,
}

impl ProjectCatalog {
    pub(crate) fn new(paths: &DataPaths) -> Result<Self> {
        let optional_projects = Projects::new(
            ProjectSets::new(&paths.project_dir).context("Loading optional projects")?,
        )?;
        let mandatory_projects = Projects::new(
            ProjectSets::new(&paths.focus_dir).context("Loading mandatory projects")?,
        )?;
        Ok(Self {
            optional_projects,
            mandatory_projects,
        })
    }
}
