use anyhow::{bail, Result};
use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
};
use tracing::{debug, error};

use super::*;

/// Project is an odd structure. It aggregates ProjectSets. This structure is meant to be constructed using the `TryFrom<ProjectSets>` implementation, which constructs a unified forward index of project name to project while keeping track of which sets projects were defined in and preventing duplicates in the flat namespace.
#[derive(Default, Debug)]
pub struct Projects {
    /// Underlying maps project names to instances of the Project structure.
    pub underlying: HashMap<String, Project>,

    /// Sources maps project names to name of the project set they were defined in.
    pub sources: HashMap<String, String>,
}

impl Display for Projects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sorted_projects = BTreeSet::from_iter(self.underlying.values());
        debug!(?self, ?sorted_projects, "Display");
        for project in sorted_projects {
            writeln!(
                f,
                "{:<48} {} ({} targets)",
                project.name,
                project.description,
                project.targets.len()
            )?;
        }
        Ok(())
    }
}
// TODO(wilhelm): Reduce duplication of the keys of these tables by introducing an intermediate token table.

impl Projects {
    pub fn extend(&mut self, project_set_name: &str, project_set: &ProjectSet) -> Result<()> {
        for project in project_set.projects.iter() {
            let project_name = project.name.clone();
            if let Some(previous_definition) = self
                .underlying
                .insert(project_name.clone(), project.clone())
            {
                let previously_defined_in = self.sources.get(&project_name)
                    .expect("Failed to determine where of previously-defined project {} was defined. This is a bug.");
                error!(?previous_definition, new_definition = ?project, "Conflicting projects detected");
                bail!(
                    "Duplicate project '{}' encountered in set {} (previously defined in {})",
                    &project_name,
                    project_set_name,
                    previously_defined_in
                );
            }

            self.sources
                .insert(project_name, project_set_name.to_owned());
        }

        Ok(())
    }

    // pub fn from(p: ProjectSets)
    pub fn is_mandatory(project: &Project) -> bool {
        project.mandatory
    }

    pub fn is_selectable(project: &Project) -> bool {
        !Self::is_mandatory(project)
    }
}

impl TryFrom<ProjectSets> for Projects {
    type Error = anyhow::Error;

    fn try_from(value: ProjectSets) -> Result<Self, Self::Error> {
        let mut projects = Self::default();
        for (project_set_name, project_set) in value.underlying().iter() {
            projects.extend(project_set_name.as_str(), project_set)?;
        }

        Ok(projects)
    }
}

impl TryFrom<&Projects> for TargetSet {
    type Error = anyhow::Error;

    fn try_from(value: &Projects) -> Result<Self, Self::Error> {
        let mut set = TargetSet::new();
        for project in value.underlying.values() {
            set.extend(TargetSet::try_from(project)?);
        }
        Ok(set)
    }
}
