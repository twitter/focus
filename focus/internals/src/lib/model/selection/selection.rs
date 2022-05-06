use anyhow::{Context, Result};
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
};

use super::*;

/// A structure representing the current selection in memory. Instead of serializing this structure, a PersistedSelection is stored to disk. In addition to that structure being simpler to serialize, the indirection allows for updates to the underlying project definitions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub projects: HashSet<Project>,
    pub targets: HashSet<Target>,
}

impl Selection {
    pub fn from_persisted_selection(
        persisted_selection: PersistedSelection,
        projects: &Projects,
    ) -> Result<Self> {
        let mut selection = Selection::default();
        let operations = Vec::<Operation>::try_from(persisted_selection)
            .context("Structuring a persisted selection as a set of operations")?;
        selection
            .apply_operations(&operations, projects)
            .context("Creating a selection from its persisted form")?;
        Ok(selection)
    }

    pub fn apply_operations(
        &mut self,
        operations: &Vec<Operation>,
        projects: &Projects,
    ) -> Result<OperationResult> {
        let mut processor = SelectionOperationProcessor {
            selection: self,
            projects,
        };
        processor.process(operations)
    }
}

impl TryFrom<&Selection> for TargetSet {
    type Error = anyhow::Error;

    fn try_from(value: &Selection) -> Result<Self, Self::Error> {
        let mut set = value.targets.clone();
        for project in value.projects.iter() {
            set.extend(TargetSet::try_from(project)?);
        }
        Ok(set)
    }
}

impl Display for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "--- Projects ---")?;
        let sorted_projects =
            BTreeSet::<Project>::from_iter(self.projects.iter().filter_map(|project| {
                if project.mandatory {
                    None
                } else {
                    Some(project.to_owned())
                }
            }));

        if sorted_projects.is_empty() {
            writeln!(f, "None selected.")?;
        } else {
            let longest_project_name = sorted_projects
                .iter()
                .fold(0_usize, |highest, project| project.name.len().max(highest));
            for project in sorted_projects.iter() {
                let mut padded_project_name = String::from(&project.name);
                padded_project_name.extend(
                    " ".chars()
                        .cycle()
                        .take(longest_project_name - project.name.len()),
                );

                writeln!(
                    f,
                    "{}   {} ({} {})",
                    padded_project_name,
                    project.description,
                    project.targets.len(),
                    if project.targets.len() == 1 {
                        "target"
                    } else {
                        "targets"
                    }
                )?;
            }
        }
        writeln!(f)?;

        writeln!(f, "--- Targets ---")?;
        let sorted_targets =
            BTreeSet::<String>::from_iter(self.targets.iter().map(|target| target.to_string()));
        if sorted_targets.is_empty() {
            writeln!(f, "None selected.")?;
        } else {
            for target in sorted_targets.iter() {
                writeln!(f, "{}", target)?;
            }
        }

        Ok(())
    }
}
