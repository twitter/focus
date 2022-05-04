use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectSet {
    pub projects: HashSet<Project>,
}
