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
