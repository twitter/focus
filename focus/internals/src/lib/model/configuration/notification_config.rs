// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum NotificationCategory {
    /// Build graph state notifications
    BuildGraphState,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NotificationConfig {
    /// Notification categories to suppress
    pub suppressed_categories: BTreeSet<NotificationCategory>,
}

impl NotificationConfig {
    pub fn is_allowed(&self, category: NotificationCategory) -> bool {
        !self.suppressed_categories.contains(&category)
    }
}
