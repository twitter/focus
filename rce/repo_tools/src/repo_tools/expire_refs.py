import os

import pygit2 as pg2
from loguru import logger
from pygit2 import Repository
from repo_tools.git.consts import REFS_HEADS_MASTER
import arrow

from . import git
from .git import refs, Repo

DEFAULT_AGE = arrow.utcnow().shift(days=-90)


def expire_old_refs(repo: Repository) -> None:
  logger.info(f"expiring refs from before {DEFAULT_AGE.format(arrow.FORMAT_RFC3339)}")
  count = 0
  with git.UpdateRef() as ur:
    for r in repo.references:
      commit = repo.lookup_reference(r).peel(pg2.GIT_OBJ_COMMIT)
      ctime = arrow.get(commit.author.time)
      if ctime < DEFAULT_AGE:
        count += 1
        ur.delete(r)
    ur.run(repo.path)
  logger.info(f"expired {count} refs")


def expire_refs_with_merge_base_older_than_target(repo: Repo) -> None:
  master_commit = repo.pg2repo.lookup_reference(REFS_HEADS_MASTER).peel(pg2.GIT_OBJ_COMMIT)
  with git.UpdateRef() as ur:
    for ref, commit in repo.iter_ref_and_commit():
      merge_base = repo.merge_base(master_commit.hex, commit.hex_id)
      if merge_base is not None and merge_base.commit_time < DEFAULT_AGE:
        ur.delete(ref.name)
    ur.run(repo.path)


def main() -> None:
  git_dir = os.environ.get("REPO_TOOLS_GIT_DIR", "/repos/source/scratch/full.git")
  repo = Repository(git_dir)
  expire_old_refs(repo)
  expire_refs_with_merge_base_older_than_target(Repo.wrap(repo))
  refs.pack(repo.path)


if __name__ == '__main__':
  main()
