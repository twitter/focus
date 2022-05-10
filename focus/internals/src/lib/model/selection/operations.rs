use std::collections::HashSet;

use anyhow::Result;

use super::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Disposition {
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
    pub disposition: Disposition,
    pub underlying: Underlying,
}

impl From<(Disposition, String)> for Operation {
    fn from(parameters: (Disposition, String)) -> Self {
        let (disposition, item) = parameters;

        let underlying = if let Ok(target) = crate::target::Target::try_from(item.as_str()) {
            Underlying::Target(target)
        } else {
            Underlying::Project(item)
        };

        Self {
            disposition,
            underlying,
        }
    }
}

#[derive(Debug, Default)]
pub struct OperationProcessorResult {
    pub added: HashSet<Underlying>,
    pub removed: HashSet<Underlying>,
    pub absent: HashSet<Underlying>,
    pub ignored: HashSet<Underlying>,
}

impl OperationProcessorResult {
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

pub trait OperationProcessor {
    fn process(&mut self, operations: &Vec<Operation>) -> Result<OperationProcessorResult>;
}

#[cfg(test)]
mod testing {
    use super::*;

    #[test]
    fn operation_from() {
        assert_eq!(
            Operation::from((Disposition::Add, String::from("bazel://a/b:*"))),
            Operation {
                disposition: Disposition::Add,
                underlying: Underlying::Target(Target::try_from("bazel://a/b:*").unwrap())
            }
        );

        assert_eq!(
            Operation::from((Disposition::Remove, String::from("foo"))),
            Operation {
                disposition: Disposition::Remove,
                underlying: Underlying::Project(String::from("foo"))
            }
        );
    }
}
