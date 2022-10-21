// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Borrow;

use crate::target::Label;

pub fn make_set(labels: impl IntoIterator<Item = impl Borrow<Label>>) -> String {
    format!(
        "set({})",
        labels
            .into_iter()
            .map(|label| label.borrow().to_string())
            .map(|label| quote_target_name(&label))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Escape any characters with special meaning to Bazel. For example, by
/// default, Bazel will try to lex curly braces (`{}`) as part of a
/// different token.
pub fn quote_target_name(target_name: &str) -> String {
    format!("\"{}\"", target_name)
}
