// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use super::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct AddOptions {
    pub unroll: bool,
}
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RemoveOptions {
    pub all: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum OperationAction {
    Add(AddOptions),
    Remove(RemoveOptions),
}

impl OperationAction {
    /// Set default options for OperationAction::Add
    pub fn default_add() -> OperationAction {
        OperationAction::Add(AddOptions { unroll: false })
    }

    /// Set default options for OperationAction::Remove
    pub fn default_remove() -> OperationAction {
        OperationAction::Remove(RemoveOptions { all: false })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Underlying {
    Target(Target),
    Project(String),
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Operation {
    pub action: OperationAction,
    pub underlying: Underlying,
}

impl Operation {
    pub fn new(action: OperationAction, string_repr: impl AsRef<str>) -> Self {
        let underlying = if let Ok(target) = crate::target::Target::try_from(string_repr.as_ref()) {
            Underlying::Target(target)
        } else {
            Underlying::Project(string_repr.as_ref().into())
        };

        Self { action, underlying }
    }
}

#[derive(Debug, Default)]
pub struct OperationResult {
    pub added: HashSet<Underlying>,
    pub removed: HashSet<Underlying>,
    pub absent: HashSet<Underlying>,
    pub ignored: HashSet<Underlying>,
}

impl OperationResult {
    /// The number of projects and targets affected by processing the operations.
    pub fn change_count(&self) -> usize {
        self.added.len() + self.removed.len()
    }

    /// Did processing the operations change the selection?
    pub fn changed(&self) -> bool {
        self.change_count() > 0
    }

    /// Were the operations processed successfully?
    pub fn is_success(&self) -> bool {
        self.absent.is_empty()
    }
}

#[cfg(test)]
mod testing {
    use super::*;

    #[test]
    fn operation_new() {
        assert_eq!(
            Operation::new(OperationAction::default_add(), "bazel://a/b:*"),
            Operation {
                action: OperationAction::default_add(),
                underlying: Underlying::Target(Target::try_from("bazel://a/b:*").unwrap())
            }
        );

        assert_eq!(
            Operation::new(OperationAction::default_remove(), "foo"),
            Operation {
                action: OperationAction::default_remove(),
                underlying: Underlying::Project(String::from("foo"))
            }
        );
    }
}
