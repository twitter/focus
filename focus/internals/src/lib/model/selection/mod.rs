// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

mod project;
pub use project::resolve_targets_for_project;
pub use project::Project;
pub(crate) use project::ProjectCatalog;
use project::ProjectIndex;

#[allow(clippy::module_inception)]
mod selection;
pub use selection::Selection;
pub(crate) use selection::SelectionManager;

use super::data_paths::DataPaths;

mod operations;
pub use operations::Operation;
pub use operations::OperationAction;
pub use operations::OperationResult;
pub(crate) use operations::Underlying;

#[cfg(test)]
mod testing;

pub(crate) use super::repo::Repo;
pub(crate) use super::repo::WorkingTree;
pub(crate) use crate::model::persistence::FileBackedCollection;
pub use crate::model::persistence::{load_model, store_model};
pub(crate) use crate::target::Target;
pub(crate) use crate::target::TargetError;
pub(crate) use crate::target::TargetSet;
