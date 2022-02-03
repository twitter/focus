use std::{fs::File, io::Write, process::Stdio, sync::Arc, collections::HashSet};

use focus_internals::{
    app::App,
    util::{
        git_helper,
        sandbox_command::{SandboxCommand, SandboxCommandOutput},
    },
};
use log::debug;

use anyhow::{Context, Result};
use chrono::{offset, Date, DateTime, FixedOffset, NaiveDate, NaiveDateTime, ParseError};
use git2::{Commit, Oid, Repository, Time};

static DATE_FORMAT: &str = "%Y-%m-%d";

pub fn parse_date(s: String) -> Result<DateTime<FixedOffset>, ParseError> {
    NaiveDate::parse_from_str(s.as_str(), DATE_FORMAT)
        .map(|nd| {
            Date::from_utc(nd, offset::FixedOffset::east(0))
                .and_hms(0, 0, 0)
        })
}

fn git_time_to_date_time(t: Time) -> DateTime<FixedOffset> {
    DateTime::from_utc(
        NaiveDateTime::from_timestamp(t.seconds(), 0 /* nanos */),
        FixedOffset::east(t.offset_minutes() * 60),
    )
}


static MASTER_NAME: &str = "refs/heads/master";

fn find_merge_base_commit(repo: &Repository, a: Oid, b: Oid) -> Result<Commit> {
    let mb_oid = repo.merge_base(a, b).context("merge_base")?;
    repo.find_commit(mb_oid)
        .context("find_commit of merge base")
}

#[derive(Debug)]
pub struct PartitionedRefNames {
    pub current: Vec<String>,
    pub expired: Vec<String>,
}

impl PartitionedRefNames {
    fn new() -> PartitionedRefNames {
        PartitionedRefNames { current: Vec::new(), expired: Vec::new() }
    }
}


pub fn partition_refs(
    repo: &Repository,
    cutoff: DateTime<FixedOffset>,
    check_merge_base: bool,
) -> Result<PartitionedRefNames> {
    let refs = repo.references().context("opening repo.references")?;

    let master_ref = repo
        .find_reference(MASTER_NAME)
        .context("finding master ref")?;
    let master_commit = master_ref
        .peel_to_commit()
        .context("peel master to commit")?;

    let mut partitioned = PartitionedRefNames::new();
    for rref in refs {
        let r = rref.context("unwrapping Reference")?;
        if r.is_tag() || r.is_branch() {
            let commit = r.peel_to_commit().context("peeling ref to commit")?;
            let ref_name = r.name().unwrap().to_string();

            let auth_time = git_time_to_date_time(commit.author().when());
            if auth_time < cutoff {
                partitioned.expired.push(ref_name);
                continue
            }
            // if the merge base of the ref with master is before the cutoff date, then don't
            // consider it current. If the ref does not share a merge base with master, it's
            // also not considered current
            else if check_merge_base {
                for mb_commit in find_merge_base_commit(&repo, master_commit.id(), commit.id()).ok()
                {
                    let mb_commit_time = git_time_to_date_time(mb_commit.author().when());
                    if mb_commit_time < cutoff {
                        partitioned.expired.push(ref_name.clone());
                        continue;
                    }
                }
            }
            partitioned.current.push(ref_name);
        }
    }

    Ok(partitioned)
}

/// Goes through 'names' and detects if there are duplicates that differ only by case.
/// This causes issues on macos which has a case insensitive filesystem, while linux
/// has a case *sensitive* filesystem, when trying to delete refs.
fn delete_case_conflict_refs(repo: &Repository, names: Vec<String>) -> Result<Vec<String>> {
    debug!("cleaning up case conflict ref names");

    let mut lower_set: HashSet<String> = HashSet::new();

    let mut ok_names: Vec<String> = Vec::new();

    for name in names {
        if !lower_set.insert(name.to_lowercase()) {
            debug!("deleting case conflict ref name: {}", name);
            // this is a conflict so delete this ref here so we don't run into
            // a problem when we do the bulk deletion
            let mut rf = repo.find_reference(&name).context("find conflicting ref")?;
            rf.delete().context("deleting conflicting ref")?;
        } else {
            ok_names.push(name);
        }
    }

    Ok(ok_names)
}

pub fn expire_old_refs(
    repo: &Repository,
    cutoff: DateTime<FixedOffset>,
    check_merge_base: bool,
    use_transaction: bool,
    app: Arc<App>,
) -> Result<()> {
    let sandbox = app.sandbox();

    let ref_file_path = {
        let (mut ref_file, ref_file_path) = sandbox.create_file(Some("update-refs"), None)?;

        let xs = {
            let partitioned = partition_refs(&repo, cutoff, check_merge_base).context("collecting expired ref names")?;
            delete_case_conflict_refs(&repo, partitioned.expired)?
        };

        let mut content: Vec<String> = xs
            .iter()
            .map(|ref_name| {
                format!("delete {}\x00\x00", ref_name)
            })
            .collect();

        if use_transaction {
            content.insert(0, "start\x00".to_string());
            content.push("prepare\x00".to_string());
            content.push("commit\x00".to_string());
        }

        ref_file
            .write_all(content.join("").as_bytes())
            .context("writing content")?;
        ref_file.sync_data().context("syncing data")?;
        ref_file_path
    };

    let ref_file = File::open(ref_file_path).context("re-opening the ref file")?;
    let (mut cmd, scmd) = SandboxCommand::new_with_handles(
        "expire refs with update-ref".to_owned(),
        git_helper::git_binary(),
        Some(Stdio::from(ref_file)),
        None,
        None,
        app,
    )?;

    scmd.ensure_success_or_log(
        cmd.current_dir(repo.workdir().unwrap_or(repo.path()))
            .arg("update-ref")
            .arg("--stdin")
            .arg("-z"),
        SandboxCommandOutput::All,
        "expiring refs",
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::{FixedOffset, DateTime};
    use crate::{refs::parse_date, subcommands::refs::git_time_to_date_time};

    #[test]
    fn test_parse_date() -> Result<()> {
        let data: Vec<(String, DateTime<FixedOffset>)> = vec![
            ("2022-01-02", "2022-01-02T00:00:00-00:00"),
            ("2022-03-05", "2022-03-05T00:00:00-00:00"),
        ].iter()
        .map(|(a, b)| (a.to_string(), DateTime::parse_from_rfc3339(b).unwrap()))
        .collect();

        for (a, b) in data {
            assert_eq!(parse_date(a)?, b);
        }

        Ok(())
    }

    #[test]
    fn test_git_time_to_date_time() -> Result<()> {
        let dt = DateTime::parse_from_rfc3339("2022-02-07T12:34:56-05:00")?;
        let expected_unix_time: i64 = 1644255296;
        let git_time = git2::Time::new(expected_unix_time, -(5 * 60));

        assert_eq!(git_time_to_date_time(git_time), dt);

        Ok(())
    }
}
