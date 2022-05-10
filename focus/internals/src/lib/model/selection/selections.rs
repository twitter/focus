use std::{
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use tracing::{debug, error};

use super::*;

pub struct Selections {
    selection_path: PathBuf,
    pub optional_projects: Projects,
    pub mandatory_projects: Projects,
    selection: Selection,
}

impl Selections {
    pub fn from_repo(repo: &Repo) -> Result<Self> {
        let working_tree = repo
            .working_tree()
            .ok_or_else(|| anyhow::anyhow!("The repo must have a working tree"))?;

        let paths = DataPaths::from_working_tree(working_tree)?;
        let optional_projects = Projects::new(
            ProjectSets::new(&paths.project_dir).context("Loading optional projects")?,
        )?;
        let mandatory_projects = Projects::new(
            ProjectSets::new(&paths.focus_dir).context("Loading mandatory projects")?,
        )?;

        Self::new(&paths.selection_file, optional_projects, mandatory_projects)
    }

    fn new(
        selection_path: &dyn AsRef<Path>,
        optional_projects: Projects,
        mandatory_projects: Projects,
    ) -> Result<Self> {
        let mut instance = Self {
            selection_path: selection_path.as_ref().to_owned(),
            optional_projects,
            mandatory_projects,
            selection: Default::default(),
        };
        instance.reload()?;
        Ok(instance)
    }

    /// Load a selection from the given `path` using project definitions from `projects`.
    fn load(path: &dyn AsRef<Path>, projects: &Projects) -> Result<Selection> {
        let persisted_selection = load_model(path).context("Loading persisted selection")?;
        Selection::from_persisted_selection(persisted_selection, projects)
    }

    /// Load the selection from disk.
    pub fn reload(&mut self) -> Result<()> {
        let selection: Selection = Self::load(&self.selection_path, &self.optional_projects)?;
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
            .mandatory_projects
            .underlying
            .values()
            .cloned()
            .collect::<Vec<Project>>();
        debug!(mandatory = ?mandatory_projects, "Mandatory projects");
        selection.projects.extend(mandatory_projects);
        Ok(selection)
    }

    pub fn mutate(
        &mut self,
        action: OperationAction,
        projects_and_targets: &Vec<String>,
    ) -> Result<bool> {
        let operations = projects_and_targets
            .iter()
            .map(|value| Operation::new(action, value.clone()))
            .collect::<Vec<Operation>>();
        let result = self.process(&operations)?;
        if !result.is_success() {
            bail!("Failed to update the selection");
        }
        Ok(result.changed())
    }

    pub fn process(&mut self, operations: &Vec<Operation>) -> Result<OperationResult> {
        let mut selection = self.selection.clone();
        let mut processor = SelectionOperationProcessor {
            selection: &mut selection,
            projects: &self.optional_projects,
        };
        match processor.process(operations) {
            Ok(result) => {
                if result.is_success() {
                    self.selection = selection;
                } else {
                    error!("The selection will not be updated because an error occured while applying the requested changes");
                }
                Ok(result)
            }
            Err(e) => Err(e),
        }
    }
}
