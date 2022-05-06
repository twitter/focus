use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use tracing::{debug, error};

use super::*;

pub struct Selections {
    selection_path: PathBuf,
    pub optional_projects: Projects,
    pub mandatory_projects: Projects,
    selection: RefCell<selection::Selection>,
}

impl Selections {
    pub fn new(
        selection_path: &dyn AsRef<Path>,
        optional_projects: Projects,
        mandatory_projects: Projects,
    ) -> Result<Self> {
        let instance = Self {
            selection_path: selection_path.as_ref().to_owned(),
            optional_projects,
            mandatory_projects,
            selection: RefCell::new(Default::default()),
        };
        instance.reload()?;
        Ok(instance)
    }

    /// Load a selection from the given `path` using project definitions from `projects`.
    fn load(path: &dyn AsRef<Path>, projects: &Projects) -> Result<Selection> {
        match FileBackedModel::load::<PersistedSelection>(path) {
            Ok(persisted_selection) => {
                let mut selection: selection::Selection = Default::default();
                let mut processor = SelectionOperationProcessor {
                    selection: &mut selection,
                    projects,
                };
                processor.reify(persisted_selection)?;
                Ok(selection)
            }
            Err(e) => bail!(
                "Failed to load persisted selection from {}: {}",
                path.as_ref().display(),
                e
            ),
        }
    }

    /// Load the selection from disk.
    pub fn reload(&self) -> Result<()> {
        let selection: Selection = Self::load(&self.selection_path, &self.optional_projects)?;
        debug!(?selection, path = ?self.selection_path, "Reloaded selection");
        self.selection.replace(selection);
        Ok(())
    }

    /// Save the current selection to the configured `selection_path`.
    pub fn save(&self) -> Result<()> {
        let selection = self.selection.borrow().clone();
        let persisted_selection = PersistedSelection::from(&selection);
        FileBackedModel::store(&self.selection_path, &persisted_selection)?;
        debug!(?persisted_selection, path = ?self.selection_path, "Saved selection");
        Ok(())
    }

    /// Returns a Selection combining both user-selected and mandatory projects and targets.
    pub fn computed_selection(&self) -> Result<Selection> {
        let mut selection = self.selection.borrow().clone();
        debug!(selected = ?selection, "User-selected projects");
        let mandatory_projects = self
            .mandatory_projects
            .underlying
            .values().cloned()
            .collect::<Vec<Project>>();
        debug!(mandatory = ?mandatory_projects, "Mandatory projects");
        selection.projects.extend(mandatory_projects);
        Ok(selection)
    }

    pub fn mutate(
        &mut self,
        disposition: Disposition,
        projects_and_targets: &Vec<String>,
    ) -> Result<bool> {
        // let disposition = disposition.copy();
        let operations = projects_and_targets
            .iter()
            .map(|value| Operation::from((disposition, value.clone())))
            .collect::<Vec<Operation>>();
        let result = self.process(&operations)?;
        if !result.is_success() {
            bail!("Failed to update the selection");
        }
        Ok(result.changed())
    }
}

impl OperationProcessor for Selections {
    fn process(&mut self, operations: &Vec<Operation>) -> Result<OperationProcessorResult> {
        let mut selection = self.selection.borrow().clone();
        let mut processor = SelectionOperationProcessor {
            selection: &mut selection,
            projects: &self.optional_projects,
        };
        match processor.process(operations) {
            Ok(result) => {
                if result.is_success() {
                    self.selection.replace(selection);
                } else {
                    error!("The selection will not be updated because an error occured while applying the requested changes");
                }
                Ok(result)
            }
            Err(e) => Err(e),
        }
    }
}

impl TryFrom<&Repo> for Selections {
    type Error = anyhow::Error;

    fn try_from(value: &Repo) -> Result<Self, Self::Error> {
        match value.working_tree() {
            Some(working_tree) => {
                let paths = DataPaths::try_from(working_tree)?;
                let optional_projects = Projects::try_from(
                    ProjectSets::new(&paths.project_dir).context("Loading optional projects")?,
                )?;
                let mandatory_projects = Projects::try_from(
                    ProjectSets::new(&paths.focus_dir).context("Loading mandatory projects")?,
                )?;

                Self::new(&paths.selection_file, optional_projects, mandatory_projects)
            }
            None => bail!("The repo must have a working tree"),
        }
    }
}
