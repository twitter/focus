use std::{collections::HashSet, fs::File, io::Write, process::Stdio, sync::Arc};

use focus_internals::{
    app::App,
    util::{
        git_helper,
        sandbox_command::{SandboxCommand, SandboxCommandOutput},
        time::FocusTime,
    },
};
use log::debug;

use anyhow::{Context, Result};
use git2::Repository;

/// Vec of names that should never be expired via this process
/// TODO: this should probably be in configuration rather than hardcoded here
const SAFE_BRANCH_NAMES: &[&str] = &[
    "refs/heads/main",
    "refs/heads/master",
    "refs/heads/repo.d/main",
    "refs/heads/repo.d/master",
];

mod partition {
    use std::collections::HashSet;

    use anyhow::{Context, Result};
    use focus_internals::util::time::{FocusTime, GitTime};
    use git2::{Commit, Oid, Repository};

    use super::PartitionedRefNames;

    #[derive(Debug, Clone)]
    pub(super) struct RefInfo {
        pub name: String,
        pub author_time: FocusTime,
        pub merge_base_auth_time: Option<FocusTime>,
    }

    fn head_commit(repo: &Repository) -> Result<Commit> {
        repo.head()
            .context("Finding repo HEAD")?
            .peel_to_commit()
            .context("peeling HEAD commit")
    }

    fn safe_refs() -> HashSet<&'static str> {
        super::SAFE_BRANCH_NAMES.iter().copied().collect()
    }

    fn safe_branches_and_tags(repo: &Repository) -> Result<Vec<git2::Reference>> {
        let safe_refs = safe_refs();

        let unfiltered = repo
            .references()
            .context("opening repo.references")?
            .into_iter()
            .collect::<Result<Vec<git2::Reference>, git2::Error>>()?;

        let filtered = unfiltered
            .into_iter()
            .filter(|r| (r.is_branch() || r.is_tag()))
            .filter(|r| r.name().map(|n| !safe_refs.contains(n)).unwrap_or(true))
            .collect();

        Ok(filtered)
    }

    fn find_merge_base_commit(repo: &Repository, a: Oid, b: Oid) -> Result<Commit> {
        let mb_oid = repo.merge_base(a, b).context("merge_base")?;
        repo.find_commit(mb_oid)
            .context("find_commit of merge base")
    }

    pub(super) fn collect_ref_info(repo: &Repository) -> Result<Vec<RefInfo>> {
        let repo = repo;
        let head = head_commit(repo)?;

        let mut refs: Vec<RefInfo> = Vec::new();

        for r in safe_branches_and_tags(repo)?.into_iter() {
            let commit = r.peel_to_commit().context("peeling ref to commit")?;
            let name = r.name().unwrap().to_string();
            let author_time = FocusTime::from(GitTime::from(commit.author().when()));
            let merge_base_auth_time = find_merge_base_commit(repo, head.id(), commit.id())
                .map(|mbc| FocusTime::from(mbc.author().when()))
                .ok();
            refs.push(RefInfo {
                name,
                author_time,
                merge_base_auth_time,
            })
        }

        Ok(refs)
    }

    pub(super) fn partitioned_ref_names(
        ref_infos: Vec<RefInfo>,
        cutoff: FocusTime,
        check_merge_base: bool,
    ) -> Result<PartitionedRefNames> {
        let (cur_info, expired_info): (Vec<RefInfo>, Vec<RefInfo>) =
            ref_infos.into_iter().partition(|ref_info| {
                let RefInfo {
                    name: _,
                    author_time,
                    merge_base_auth_time,
                } = ref_info;
                let auth_time = author_time;
                if *auth_time < cutoff {
                    false
                } else if check_merge_base {
                    match merge_base_auth_time {
                        Some(t) if *t < cutoff => false,
                        None => true, // merge base not found, assume its current or an orphan branch
                        _ => true,
                    }
                } else {
                    true
                }
            });

        Ok(PartitionedRefNames {
            current: cur_info.into_iter().map(|i| i.name).collect(),
            expired: expired_info.into_iter().map(|i| i.name).collect(),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PartitionedRefNames {
    pub current: Vec<String>,
    pub expired: Vec<String>,
}

impl PartitionedRefNames {
    /// convenience constructor, given a repo, the cutoff time, and the check_merge_base option,
    /// create a PartitionedRefNames instance for the repo's references.
    pub fn for_repo(repo: &Repository, cutoff: FocusTime, check_merge_base: bool) -> Result<Self> {
        partition::partitioned_ref_names(
            partition::collect_ref_info(repo)?,
            cutoff,
            check_merge_base,
        )
    }
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
            rf.delete().context("delete conflicting ref")?;
        } else {
            ok_names.push(name);
        }
    }

    Ok(ok_names)
}

pub fn expire_old_refs(
    repo: &Repository,
    cutoff: FocusTime,
    check_merge_base: bool,
    use_transaction: bool,
    app: Arc<App>,
) -> Result<()> {
    let sandbox = app.sandbox();

    let ref_file_path = {
        let (mut ref_file, ref_file_path, _) =
            sandbox.create_file(Some("update-refs"), None, None)?;

        let xs = {
            let PartitionedRefNames {
                current: _,
                expired,
            } = PartitionedRefNames::for_repo(repo, cutoff, check_merge_base)
                .context("collecting expired ref names")?;
            delete_case_conflict_refs(repo, expired)?
        };

        let mut content: Vec<String> = xs
            .iter()
            .map(|ref_name| format!("delete {}\x00\x00", ref_name))
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
        "expire refs with update-ref",
        git_helper::git_binary(),
        Some(Stdio::from(ref_file)),
        None,
        None,
        app,
    )?;

    scmd.ensure_success_or_log(
        cmd.current_dir(repo.workdir().unwrap_or_else(|| repo.path()))
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
    use std::fs;

    use anyhow::Result;
    use focus_internals::util::{git_helper::Ident, time::FocusTime};

    use crate::subcommands::testing::refs::Fixture;

    static OLD_MERGE_BASE_BRANCH_NAME: &'static str = "refs/heads/oldmergebase";
    static OLD_TIP_BRANCH_NAME: &'static str = "refs/heads/oldtip";
    static REFS_HEADS_MAIN: &'static str = "refs/heads/main";
    static FIRST_COMMIT_TIMESTAMP: &'static str = "2016-02-03T00:00:00-05:00";
    static REFS_HEADS_REPOD_MASTER: &'static str = "refs/heads/repo.d/master";

    // we want a repo that has commits like:
    //
    //      F            << oldtip
    //    /
    //   A --> B --> C   << main
    //    \
    //     D --> E       << oldmergebase
    //
    // A and F have timestamps far in the past, every thing else has a "now" timestamp
    fn setup_ref_repo(fix: &mut Fixture, ident: &Ident) -> Result<()> {
        let sig = ident.to_signature()?;

        let oid_a =
            fix.write_add_and_commit("f0", "f0", "initial commit", Some(&sig), Some(&sig))?;

        fix.checkout_b(OLD_TIP_BRANCH_NAME, oid_a)?;

        fix.write_add_and_commit("ffs", "ffs", "ffs", Some(&sig), Some(&sig))?;

        fix.checkout(REFS_HEADS_MAIN, None)?;

        let i2 = Ident {
            timestamp: FocusTime::now(),
            ..ident.clone()
        };
        let i2_sig = i2.to_signature()?;

        let write_and_commit = |f: &mut Fixture, s: &str| -> Result<git2::Oid> {
            f.write_add_and_commit(s, s, s, Some(&i2_sig), Some(&i2_sig))
        };

        let _oid_b = write_and_commit(fix, "f1")?;
        let _oid_c = write_and_commit(fix, "f2")?;

        fix.checkout_b(OLD_MERGE_BASE_BRANCH_NAME, oid_a)?;

        let _oid_d = write_and_commit(fix, "f3")?;
        let _oid_e = write_and_commit(fix, "f4")?;

        fix.checkout(REFS_HEADS_MAIN, None)?;

        Ok(())
    }

    fn old_ident() -> Ident {
        Ident {
            name: "Carter Pewterschmidt".to_string(),
            email: "cpewterschmidt@twitter.com".to_string(),
            timestamp: FocusTime::parse_from_rfc3339(FIRST_COMMIT_TIMESTAMP)
                .expect("failed to parse date"),
        }
    }

    #[test]
    fn test_expire_based_on_merge_base() -> Result<()> {
        let mut fix = Fixture::new()?;
        let ident = old_ident();

        {
            setup_ref_repo(&mut fix, &ident)?;
        }

        let repo = fix.repo();

        let cutoff = FocusTime::now() - chrono::Duration::days(90);

        assert!(repo.find_reference(OLD_MERGE_BASE_BRANCH_NAME).is_ok());

        super::expire_old_refs(repo, cutoff, true, false, fix.app())?;

        assert!(repo.find_reference(OLD_MERGE_BASE_BRANCH_NAME).is_err());
        assert!(repo.find_reference(OLD_TIP_BRANCH_NAME).is_err());
        assert!(repo.find_reference(REFS_HEADS_MAIN).is_ok());

        Ok(())
    }

    #[test]
    fn test_expire_ignoring_merge_base() -> Result<()> {
        let mut fix = Fixture::new()?;
        let ident = old_ident();
        {
            setup_ref_repo(&mut fix, &ident)?;
        }

        let repo = fix.repo();
        let cutoff = FocusTime::now() - chrono::Duration::days(90);

        super::expire_old_refs(repo, cutoff, false, false, fix.app())?;

        assert!(repo.find_reference(OLD_TIP_BRANCH_NAME).is_err());
        assert!(repo.find_reference(OLD_MERGE_BASE_BRANCH_NAME).is_ok());
        assert!(repo.find_reference(REFS_HEADS_MAIN).is_ok());

        Ok(())
    }

    #[test]
    fn test_expire_leaves_repod_master_untouched() -> Result<()> {
        let mut fix = Fixture::new()?;
        let ident = old_ident();
        {
            setup_ref_repo(&mut fix, &ident)?;
        }

        {
            // set head to unborn branch
            let repo = fix.repo();
            repo.set_head("refs/heads/repo.d/master")?;

            // cleanup existing files
            let workdir = repo.workdir().unwrap().to_owned();
            for res in fs::read_dir(workdir)? {
                let entry = res?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    fs::remove_dir_all(entry.path())?;
                } else {
                    fs::remove_file(entry.path())?;
                }
            }
        }

        let cutoff = FocusTime::now() - chrono::Duration::days(90);

        // create a new commit
        let oid = {
            let sig = ident.to_signature()?;
            fix.write_add_and_commit(
                "stuff.sh",
                "this is stuff",
                "stuff for repo.d",
                Some(&sig),
                Some(&sig),
            )
        }?;

        {
            let repo = fix.repo();

            let repod_commit = repo.find_commit(oid)?;

            // make sure this commit is older than the cutoff so we know we're actually
            // testing the right thing.
            assert!(FocusTime::from(repod_commit.author().when()) < cutoff);

            // create a ref with the same name that HEAD points to, making it a "born" branch
            repo.reference(REFS_HEADS_REPOD_MASTER, oid, true, "")?;
        }

        // switch head back to main
        fix.checkout(REFS_HEADS_MAIN, Some(git2::ResetType::Hard))?;

        {
            let repo = fix.repo();
            super::expire_old_refs(repo, cutoff, false, false, fix.app())?;

            assert!(repo.find_reference(OLD_TIP_BRANCH_NAME).is_err());
            assert!(repo.find_reference(OLD_MERGE_BASE_BRANCH_NAME).is_ok());
            assert!(repo.find_reference(REFS_HEADS_MAIN).is_ok());
            assert!(repo.find_reference(REFS_HEADS_REPOD_MASTER).is_ok());
        }

        Ok(())
    }
}
