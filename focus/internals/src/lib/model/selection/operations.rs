use std::collections::HashSet;

use anyhow::Result;

use super::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum OperationAction {
    Add,
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Underlying {
    Target(Target),
    Project(String),
}

#[derive(Debug, PartialEq, Hash)]
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
            Operation::new(OperationAction::Add, "bazel://a/b:*"),
            Operation {
                action: OperationAction::Add,
                underlying: Underlying::Target(Target::try_from("bazel://a/b:*").unwrap())
            }
        );

        assert_eq!(
            Operation::new(OperationAction::Remove, "foo"),
            Operation {
                action: OperationAction::Remove,
                underlying: Underlying::Project(String::from("foo"))
            }
        );
    }
}
