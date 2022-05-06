use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
};

use super::*;

/// A structure representing the current 6 in memory. Instead of serializing this structure, a PersistedSelection is stored to disk. In addition to that structure being simpler to serialize, it also allows for updates to the underlying project definitions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub projects: HashSet<Project>,
    pub targets: HashSet<Target>,
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
            for project in sorted_projects.iter() {
                writeln!(
                    f,
                    "{:<48} {} ({} targets)",
                    project.name,
                    project.description,
                    project.targets.len()
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
