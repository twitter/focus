mod project;
pub(crate) use project::Project;
pub(crate) use project::ProjectCatalog;
use project::ProjectIndex;

mod selection;
pub(crate) use selection::SelectionManager;

mod data_paths;
use data_paths::DataPaths;

mod operations;
pub(crate) use operations::Operation;
pub(crate) use operations::OperationAction;
pub(crate) use operations::OperationResult;
pub(crate) use operations::Underlying;

#[cfg(test)]
mod testing;

pub(crate) use super::repo::Repo;
pub(crate) use super::repo::WorkingTree;
pub(crate) use crate::model::persistence::FileBackedCollection;
pub(crate) use crate::model::persistence::{load_model, store_model};
pub(crate) use crate::target::Target;
pub(crate) use crate::target::TargetError;
pub(crate) use crate::target::TargetSet;
