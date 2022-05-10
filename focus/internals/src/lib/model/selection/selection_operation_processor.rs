


use anyhow::Context;
use tracing::error;
use tracing::warn;

use tracing::debug;

use anyhow::Result;

use super::*;

pub(crate) struct SelectionOperationProcessor<'processor> {
    pub selection: &'processor mut Selection,
    pub projects: &'processor Projects,
}

impl<'processor> OperationProcessor for SelectionOperationProcessor<'processor> {
    fn process(&mut self, operations: &Vec<Operation>) -> Result<OperationProcessorResult> {
        let mut result: OperationProcessorResult = Default::default();

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

impl<'processor> SelectionOperationProcessor<'processor> {
    pub(crate) fn reify(&mut self, persisted_selection: PersistedSelection) -> Result<()> {
        let ops: Vec<Operation> = persisted_selection
            .try_into()
            .context("Failed to convert persisted selections to an operation stream")?;
        self.process(&ops)?;
        Ok(())
    }
}
