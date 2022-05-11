use anyhow::bail;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use tracing::error;

use std::collections::HashSet;

use super::*;

/// A structure to store the names of selected projects and targets. Converted from the fully-featured in-memory representation Selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedSelection {
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
