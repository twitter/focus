use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    path::{Path, PathBuf},
};
use tracing::{debug, error, warn};

use super::*;

/// A structure representing the current selection in memory. Instead of serializing this structure, a PersistedSelection is stored to disk. In addition to that structure being simpler to serialize, the indirection allows for updates to the underlying project definitions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub projects: HashSet<Project>,
    pub targets: HashSet<Target>,
}

impl Selection {
    fn from_persisted_selection(
        persisted_selection: PersistedSelection,
        projects: &ProjectIndex,
    ) -> Result<Self> {
        let mut selection = Selection::default();
        let operations = Vec::<Operation>::try_from(persisted_selection)
            .context("Structuring a persisted selection as a set of operations")?;
        selection
            .apply_operations(&operations, projects)
            .context("Creating a selection from its persisted form")?;
        Ok(selection)
    }

    fn apply_operations(
        &mut self,
        operations: &[Operation],
        projects: &ProjectIndex,
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
        let sorted_projects = {
            let mut projects: Vec<_> = self
                .projects
                .iter()
                .filter(|&project| !project.mandatory)
                .collect();
            projects.sort_unstable_by_key(|&project| project.name.as_str());
            projects
        };

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

pub(crate) struct SelectionOperationProcessor<'processor> {
    pub selection: &'processor mut Selection,
    pub projects: &'processor ProjectIndex,
}

impl<'processor> SelectionOperationProcessor<'processor> {
    pub fn process(&mut self, operations: &[Operation]) -> Result<OperationResult> {
        let mut result: OperationResult = Default::default();

        for operation in operations {
            debug!(?operation, "Processing operation");
            match (&operation.action, &operation.underlying) {
                (OperationAction::Add, Underlying::Target(target)) => {
                    if self.selection.targets.insert(target.clone()) {
                        result.added.insert(operation.underlying.clone());
                        debug!(?target, "Target added to selection")
                    } else {
                        result.ignored.insert(operation.underlying.clone());
                        debug!(?target, "Target already in selection")
                    }
                }
                (OperationAction::Add, Underlying::Project(name)) => {
                    match self.projects.underlying.get(name.as_str()) {
                        Some(project) => {
                            if self.selection.projects.insert(project.clone()) {
                                result.added.insert(operation.underlying.clone());
                                debug!(?project, "Project added to selection");
                            } else {
                                result.ignored.insert(operation.underlying.clone());
                                debug!(?project, "Project already in selection");
                            }
                        }
                        None => {
                            warn!(%name, "Project to be added was not found");
                            result.absent.insert(operation.underlying.clone());
                        }
                    }
                }
                (OperationAction::Remove, Underlying::Target(target)) => {
                    if self.selection.targets.remove(target) {
                        debug!(?target, "Target removed from selection");
                        result.removed.insert(operation.underlying.clone());
                    } else {
                        warn!(?target, "Target to be removed was not in selection");
                        result.ignored.insert(operation.underlying.clone());
                    }
                }
                (OperationAction::Remove, Underlying::Project(name)) => {
                    match self.projects.underlying.get(name) {
                        Some(project) => {
                            if self.selection.projects.remove(project) {
                                debug!(?project, "Project removed from selection");
                                result.removed.insert(operation.underlying.clone());
                            } else {
                                warn!(%name, "Project to be removed was not in selection");
                                result.ignored.insert(operation.underlying.clone());
                            }
                        }
                        None => {
                            error!(%name, "Project to be removed is not a defined project");
                            result.absent.insert(operation.underlying.clone());
                        }
                    }
                }
            }
        }

        Ok(result)
    }
}

/// SelectionManager maintains the current selection within a repository. It also provides access to projects defined in the repository via the `project_catalog()` method and associated structure.
pub(crate) struct SelectionManager {
    /// The path where the selection is stored.
    selection_path: PathBuf,
    /// The currently selected projects.
    selection: Selection,
    /// A catalog of defined projects.
    project_catalog: ProjectCatalog,
}

impl SelectionManager {
    pub fn from_repo(repo: &Repo) -> Result<Self> {
        let working_tree = repo
            .working_tree()
            .ok_or_else(|| anyhow::anyhow!("The repo must have a working tree"))?;
        let paths = DataPaths::from_working_tree(working_tree)?;
        let project_catalog = ProjectCatalog::new(&paths)?;
        Self::new(&paths.selection_file, project_catalog)
    }

    fn new(selection_path: impl AsRef<Path>, project_catalog: ProjectCatalog) -> Result<Self> {
        let mut instance = Self {
            selection_path: selection_path.as_ref().to_owned(),
            selection: Default::default(),
            project_catalog,
        };
        instance.reload()?;
        Ok(instance)
    }

    /// Load a selection from the given `path` using project definitions from `projects`.
    fn load(path: impl AsRef<Path>, projects: &ProjectIndex) -> Result<Selection> {
        let persisted_selection = load_model(path).context("Loading persisted selection")?;
        Selection::from_persisted_selection(persisted_selection, projects)
    }

    /// Load the selection from disk.
    pub fn reload(&mut self) -> Result<()> {
        let selection: Selection = Self::load(
            &self.selection_path,
            &self.project_catalog.optional_projects,
        )?;
        debug!(?selection, path = ?self.selection_path, "Reloaded selection");
        self.selection = selection;
        Ok(())
    }

    /// Save the current selection to the configured `selection_path`.
    pub fn save(&self) -> Result<()> {
        let selection = self.selection.clone();
        let persisted_selection = PersistedSelection::from(&selection);
        store_model(&self.selection_path, &persisted_selection)?;
        debug!(?persisted_selection, path = ?self.selection_path, "Saved selection");
        Ok(())
    }

    /// Returns a Selection combining both user-selected and mandatory projects and targets.
    pub fn computed_selection(&self) -> Result<Selection> {
        let mut selection = self.selection.clone();
        debug!(selected = ?selection, "User-selected projects");
        let mandatory_projects = self
            .project_catalog
            .mandatory_projects
            .underlying
            .values()
            .cloned()
            .collect::<Vec<Project>>();
        debug!(mandatory = ?mandatory_projects, "Mandatory projects");
        selection.projects.extend(mandatory_projects);
        Ok(selection)
    }

    /// Returns user-selected projects and targets.
    pub fn selection(&self) -> Result<Selection> {
        Ok(self.selection.clone())
    }

    pub fn mutate(
        &mut self,
        action: OperationAction,
        projects_and_targets: &[String],
    ) -> Result<bool> {
        let operations = projects_and_targets
            .iter()
            .map(|value| Operation::new(action, value.clone()))
            .collect::<Vec<Operation>>();
        let result = self
            .process(&operations)
            .context("Processing updates to the selection")?;
        Ok(result.changed())
    }

    pub fn process(&mut self, operations: &[Operation]) -> Result<OperationResult> {
        let mut selection = self.selection.clone();
        let result =
            selection.apply_operations(operations, &self.project_catalog.optional_projects)?;
        if result.is_success() {
            self.selection = selection;
        } else {
            error!("The selection will not be updated because an error occured while applying the requested changes");
        }
        Ok(result)
    }

    /// Get a reference to the selection manager's project catalog.
    #[must_use]
    pub fn project_catalog(&self) -> &ProjectCatalog {
        &self.project_catalog
    }
}

/// A structure to store the names of selected projects and targets. Converted from the fully-featured in-memory representation Selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PersistedSelection {
    pub projects: HashSet<String>,
    pub targets: HashSet<String>,
}

impl From<&Selection> for PersistedSelection {
    fn from(selection: &Selection) -> Self {
        let projects = selection
            .projects
            .iter()
            .filter(|&project| project.is_selectable())
            .map(|project| project.name.clone())
            .collect::<HashSet<String>>();
        let targets = selection
            .targets
            .iter()
            .map(String::from)
            .collect::<HashSet<String>>();
        Self { projects, targets }
    }
}

impl TryFrom<PersistedSelection> for Vec<Operation> {
    type Error = anyhow::Error;

    fn try_from(persisted_selection: PersistedSelection) -> Result<Self> {
        let targets = persisted_selection
            .targets
            .iter()
            .map(|repr| Target::try_from(repr.as_str()));
        let errors = targets
            .clone()
            .filter_map(|r| r.err())
            .collect::<Vec<TargetError>>();
        for error in errors.iter() {
            error!(%error, "Interpreting target failed");
        }
        if !errors.is_empty() {
            bail!("Some targets could not be interpreted: {:?}", errors)
        }

        let successful_targets = targets.filter_map(|r| r.ok());
        let target_operations = successful_targets.map(|target| Operation {
            action: OperationAction::Add,
            underlying: Underlying::Target(target),
        });

        let project_operations = persisted_selection.projects.iter().map(|name| Operation {
            action: OperationAction::Add,
            underlying: Underlying::Project(name.clone()),
        });

        Ok(project_operations.chain(target_operations).collect())
    }
}

#[cfg(test)]
mod testing {
    use super::*;
    use anyhow::Result;
    use maplit::{btreeset, hashset};

    const PROJECT_NAME_STR: &str = "a_project";
    const TARGET_STR: &str = "bazel://c:d";

    fn project() -> Project {
        Project {
            name: PROJECT_NAME_STR.to_owned(),
            description: String::from("This is a description"),
            mandatory: false,
            targets: btreeset![String::from("bazel://a:b")],
        }
    }

    fn target() -> Target {
        Target::try_from(TARGET_STR).unwrap()
    }

    fn selection() -> Selection {
        Selection {
            projects: hashset! {project()},
            targets: hashset! {target()},
        }
    }

    #[test]
    fn from_selection() -> Result<()> {
        let selection = selection();
        let persisted_selection = PersistedSelection::from(&selection);
        assert_eq!(
            persisted_selection.projects,
            hashset! {PROJECT_NAME_STR.to_owned()}
        );
        assert_eq!(
            persisted_selection.targets,
            hashset! {TARGET_STR.to_owned()}
        );

        Ok(())
    }

    #[test]
    fn operation_vec_try_from() -> Result<()> {
        let selection = selection();
        let persisted_selection = PersistedSelection::from(&selection);
        let ops = Vec::<Operation>::try_from(persisted_selection)?;
        assert_eq!(
            vec![
                Operation {
                    action: OperationAction::Add,
                    underlying: Underlying::Project(PROJECT_NAME_STR.to_owned())
                },
                Operation {
                    action: OperationAction::Add,
                    underlying: Underlying::Target(target())
                }
            ],
            ops
        );

        Ok(())
    }
}
