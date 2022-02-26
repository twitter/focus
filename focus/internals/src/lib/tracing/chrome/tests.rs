#![cfg(test)]

use super::*;
use crate::tracing::testing::*;
use anyhow::Result;

fn assert_trace_snapshot<S: AsRef<str>>(input: S) -> Result<()> {
    let trace = Trace::git_trace_from_file(&fixture_path(input.as_ref())?)?;
    insta::assert_yaml_snapshot!(input.as_ref(), trace);
    Ok(())
}

#[test]
fn test_clone_focus_trace() -> Result<()> {
    assert_trace_snapshot("clone-perf.json")
}

#[test]
fn test_status_trace() -> Result<()> {
    assert_trace_snapshot("status.json")
}

#[test]
fn test_push_trace() -> Result<()> {
    assert_trace_snapshot("push.json")
}

#[test]
fn test_reset_trace() -> Result<()> {
    assert_trace_snapshot("reset.json")
}

#[test]
fn test_reflog_trace() -> Result<()> {
    assert_trace_snapshot("reflog.json")
}

#[test]
fn test_rebase_trace() -> Result<()> {
    assert_trace_snapshot("rebase.json")
}

#[test]
fn test_branch_update_trace() -> Result<()> {
    assert_trace_snapshot("branch-update.json")
}
