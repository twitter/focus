#!/usr/bin/env python3

import io
import os
import os.path as osp
import re
import sys
import time
from collections import defaultdict
from contextlib import contextmanager
from pathlib import Path
from shlex import split as shplit
from subprocess import run
from tempfile import TemporaryFile
from typing import DefaultDict, Dict, Iterable, List, Optional

import attr
import pygit2 as git2
from loguru import logger

from . import git
from .git import refs, Ref
from .git.fs_refs import FakeRefDb, FDNameConflict
from .git.consts import REFS_HEADS, REFS_HEADS_SLASH, REFS_REMOTES_ORIGIN, REFS_REMOTES_ORIGIN_SLASH

AT_TWITTER_DOT_COM = "@twitter.com"


@attr.s(auto_attribs=True)
class Categorized(object):
  author_map: DefaultDict[str, List[Ref]] = attr.ib(factory=lambda: defaultdict(list))
  non_twitter: List[Ref] = attr.ib(factory=list)
  invalid: List[Ref] = attr.ib(factory=list)


def categorize_refs(git_dir: str) -> Categorized:
  logger.info("categorizing refs")
  cat = Categorized()

  for ref in refs.iter_refs(git_dir, pattern=REFS_HEADS):
    if ref.author_email[0].strip() == "@":
      cat.invalid.append(ref)
    elif not ref.has_valid_twitter_email:
      cat.non_twitter.append(ref)
    else:
      cat.author_map[ref.twitter_email_local].append(ref)

  return cat


def author_ns_ref(ref: str, author: str, namespace: str = "u") -> Optional[str]:
  if ref == f"{REFS_HEADS}/master":
    return None
  stripped = ref[len(REFS_HEADS) + 1:]
  i = stripped.find("/")
  if i < 0:
    return None

  nr: List[str] = []
  if stripped[:i] == author:
    nr.append(stripped)
  else:
    nr.extend([author, stripped])

  return '/'.join([REFS_HEADS, namespace, *nr])


def move_refs_from_remotes_origin_to_heads(git_dir: str) -> None:
  """the initial import of source is set up as a regular repo with a remote origin
  this function rewrites it to be a mirror of source with all refs under refs/heads
  """
  logger.info("moving refs from refs/remotes/origin to refs/heads")

  with git.UpdateRef() as uri:
    for ref in refs.iter_refs(git_dir, pattern=REFS_REMOTES_ORIGIN):
      if not ref.is_remote:
        continue

      new_ref = ref.name.replace(REFS_REMOTES_ORIGIN_SLASH, REFS_HEADS_SLASH)
      if new_ref == f"{REFS_HEADS}/master":
        continue

      uri.create(new_ref, ref.id_hex)
      uri.delete(ref.name)

    uri.delete("refs/remotes/origin/master")  # clean up this ref, as it's irrelevant now
    uri.delete("refs/heads/master")  # we don't need this ref for dev
    uri.run(git_dir)


def pack_refs(git_dir: str) -> None:
  logger.info("packing refs")
  run(["git", "-C", git_dir, "pack-refs", "--all"], check=True)


def calculate_renames_for_author(author: str, refs: List[Ref], rdb: FakeRefDb) -> None:
  for old in sorted(refs):
    new = author_ns_ref(old.name, author)
    if new is None:
      logger.debug(f"rename {old} author {author} returned None")
      continue

    tries = 0
    while tries < 3:
      tries += 1
      try:
        rdb.add_ref(Path(new), old.name)
        break
      except FDNameConflict as e:
        cr = rdb.root.joinpath(e.conflict_ref)
        # rename the original ref and try again
        cr.rename(cr.with_name(cr.name + "_"))
    else:
      raise RuntimeError(f"ran out of tries for {old} -> {new}")


def calculate_renames(d: Dict[str, List[Ref]]) -> FakeRefDb:
  logger.info("calculating renames")
  rdb = FakeRefDb()
  for author, refs in d.items():
    calculate_renames_for_author(author, refs, rdb)
  return rdb


@attr.s()
class XForm(object):
  x = attr.ib(type=DefaultDict[str, List[str]], factory=lambda: defaultdict(list))

  def add(self, old: str, new: str) -> None:
    x = self.x[old]
    x.append(old)
    self.x[new] = x
    del self.x[old]

  def items(self) -> Iterable[List[str]]:
    rv = []
    for k, v in sorted(list(self.x.items())):
      x = v + [k]
      rv.append(list(reversed(x)))
    return iter(rv)


def create_branches_under_u_author_name(
    git_dir: str, d: Dict[str, List[Ref]], xforms: XForm
) -> None:
  """creates new branches under refs/heads/u/{authorname}
  returns a list of the original refs that were renamed
  """
  logger.info("creating refs/heads -> /refs/heads/u/$author/$refname")
  renamed_orig = []

  old_to_ref: Dict[str, Ref] = {}
  for reflist in d.values():
    for r in reflist:
      old_to_ref[r.name] = r

  with git.UpdateRef() as uri:
    rdb = calculate_renames(d)
    try:
      for old, new in rdb.walk():
        uri.create(new, old_to_ref[old].id_hex)
        renamed_orig.append(old)
        xforms.add(old, new)

    finally:
      rdb.cleanup()

    uri.run(git_dir)

  logger.info("deleting original author refs")
  git.delete_refs(git_dir, renamed_orig)


def un_namespace_u_refs(git_dir: str, xforms: XForm) -> None:
  logger.info("removing 'u' namespace from author refs")
  orig: List[str] = []
  with git.UpdateRef() as uri:
    u_prefix = f"{REFS_HEADS}/u"
    for ref in refs.iter_refs(git_dir):
      if not ref.name.startswith(u_prefix):
        continue
      stripped = ref.name[len(u_prefix) + 1:]
      new_ref = f"{REFS_HEADS}/{stripped}"
      uri.create(new_ref, ref.id_hex)
      orig.append(ref.name)
      xforms.add(ref.name, new_ref)
    uri.run(git_dir)
  logger.info("cleaning up 'u' namespaced refs")
  git.delete_refs(git_dir, orig)


def create_unknown_namespace_for_non_twitter_owners(
    git_dir: str, unknown_refs: List[Ref], xforms: XForm
) -> None:
  renamed_orig: List[str] = []
  with git.UpdateRef() as uri:
    logger.info("creating new unknown namespaced refs")
    for ref in unknown_refs:
      stripped = ref.name[len(REFS_HEADS) + 1:]
      new_ref = '/'.join([REFS_HEADS, "unknown", stripped])
      uri.create(new_ref, ref.id_hex)
      renamed_orig.append(ref.name)
      xforms.add(ref.name, new_ref)
    uri.run(git_dir)

  logger.info("deleting original unknown refs")
  git.delete_refs(git_dir, renamed_orig)


# honestly this is a bug i haven't figured out but whatever, just move these out of the way
def move_non_u_refs_to_unknown(git_dir: str, xforms: XForm) -> None:
  logger.info("cleaning up leftovers")
  renamed_orig: List[str] = []
  with git.UpdateRef() as uri:
    for ref in refs.iter_refs(git_dir):
      if (
          not ref.startswith("refs/heads/u/") and not ref.startswith("refs/heads/unknown/") and
          not ref.startswith("refs/tags/") and ref.name != "refs/heads/master"
      ):
        stripped = ref.name[len(REFS_HEADS) + 1:]
        new_ref = '/'.join([REFS_HEADS, "unknown", stripped])
        uri.create(new_ref, ref.id_hex)
        renamed_orig.append(ref.name)
        xforms.add(ref.name, new_ref)
    uri.run(git_dir)
  git.delete_refs(git_dir, renamed_orig)


def rename(git_dir: str) -> None:
  xforms = XForm()

  move_refs_from_remotes_origin_to_heads(git_dir)

  cat = categorize_refs(git_dir)

  create_unknown_namespace_for_non_twitter_owners(git_dir, cat.non_twitter, xforms)
  create_branches_under_u_author_name(git_dir, cat.author_map, xforms)

  logger.info("deleting invalid refs")
  git.update_ref.delete(git_dir, [r.name for r in cat.invalid])

  move_non_u_refs_to_unknown(git_dir, xforms)

  un_namespace_u_refs(git_dir, xforms)
  pack_refs(git_dir)

  for xf in xforms.items():
    print('\t'.join(xf))


def main() -> None:
  git_dir = os.environ.get("REPO_TOOLS_GIT_DIR", "/repos/source/full.git")
  rename(git_dir)


if __name__ == '__main__':
  main()
