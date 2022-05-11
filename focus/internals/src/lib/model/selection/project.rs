use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ffi::OsString,
    fmt::Display,
    path::Path,
};

use serde::{Deserialize, Serialize};
use tracing::error;

use super::*;

/// A project is a collection of targets.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Project {
    /// The name of the project.
    pub name: String,

    /// A description of the project.
    pub description: String,

    /// Whether this project is mandatory. All mandatory projects defined in a repository are always in the selection.
    #[serde(default)]
    pub mandatory: bool,

    /// The targets associated with this project.
    pub targets: BTreeSet<String>,
}

impl Project {
    /// Returns whether this project is mandatory, meaning it is always present in the sparse outline and is not presented to users as being selectable.
    pub fn is_mandatory(&self) -> bool {
        self.mandatory
    }

    /// Returns whether this project should be available to select by users.
    pub fn is_selectable(&self) -> bool {
        !self.is_mandatory()
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

/// ProjectSet is a file-level container for projects.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct ProjectSet {
    pub projects: HashSet<Project>,
}

/// ProjectSetManager loads project sets from files defined in the repository.
struct ProjectSetStore(FileBackedCollection<ProjectSet>);

impl ProjectSetStore {
    pub(crate) fn new(directory: impl AsRef<Path>) -> Result<Self> {
        Ok(Self(FileBackedCollection::<ProjectSet>::new(
            directory,
            OsString::from("projects.json"),
        )?))
    }

    pub fn underlying(&self) -> &HashMap<String, ProjectSet> {
        &self.0.underlying
    }
}

/// ProjectIndex indexes projects loaded from a ProjectSetStore. When constructed, a unified forward index of project name to project is maintained, keeping track of which sets projects were defined in and preventing duplicates in the flat namespace.
#[derive(Default, Debug)]
pub(crate) struct ProjectIndex {
    /// Underlying maps project names to instances of the Project structure.
    pub underlying: HashMap<String, Project>,

    /// Sources maps project names to name of the project set file they were defined in.
    pub sources: HashMap<String, String>,
}

impl Display for ProjectIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sorted_projects = {
            let mut projects: Vec<_> = self.underlying.values().into_iter().collect();
            projects.sort_unstable_by_key(|project| project.name.as_str());
            projects
        };
        let longest_project_name = sorted_projects
            .iter()
            .fold(0_usize, |highest, &project| project.name.len().max(highest));
        for project in sorted_projects {
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
        Ok(())
    }
}
// TODO(wilhelm): Reduce duplication of the keys of these tables by introducing an intermediate token table.

impl ProjectIndex {
    fn new(manager: ProjectSetStore) -> Result<Self> {
        let mut projects = Self::default();
        for (project_set_name, project_set) in manager.underlying().iter() {
            projects.extend(project_set_name.as_str(), project_set)?;
        }

        Ok(projects)
    }

    fn extend(&mut self, project_set_name: &str, project_set: &ProjectSet) -> Result<()> {
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
}

impl TryFrom<&ProjectIndex> for TargetSet {
    type Error = anyhow::Error;

    fn try_from(value: &ProjectIndex) -> Result<Self, Self::Error> {
        let mut set = TargetSet::new();
        for project in value.underlying.values() {
            set.extend(TargetSet::try_from(project)?);
        }
        Ok(set)
    }
}

/// ProjectCatalog maintains indices of optional and mandatory projects defined in a repository.
pub(crate) struct ProjectCatalog {
    pub optional_projects: ProjectIndex,
    pub mandatory_projects: ProjectIndex,
}

impl ProjectCatalog {
    pub(crate) fn new(paths: &DataPaths) -> Result<Self> {
        let optional_projects = ProjectIndex::new(
            ProjectSetStore::new(&paths.project_dir).context("Loading optional projects")?,
        )?;
        let mandatory_projects = ProjectIndex::new(
            ProjectSetStore::new(&paths.focus_dir).context("Loading mandatory projects")?,
        )?;
        Ok(Self {
            optional_projects,
            mandatory_projects,
        })
    }
}
