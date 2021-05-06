from contextlib import contextmanager

from repo_tools.git.refs import Ref

import arrow
import attr
import pygit2 as pg2
from pygit2 import Oid
from arrow import Arrow
from typing import Generator, Iterable, Iterator, List, Optional, Tuple, Union, cast

OIDish = Union[Oid, str]


@attr.s(frozen=True, slots=True, auto_attribs=True, init=True)
class Signature(object):
  email: str
  name: str
  time: Arrow

  @classmethod
  def from_pygit2(cls, sig: pg2.Signature) -> "Signature":
    return cls(
        email=sig.email,
        name=sig.name,
        time=arrow.get(sig.time),
    )


@attr.s(frozen=True, slots=True, auto_attribs=True, init=True)
class Commit(object):
  author: Signature
  commit_time: Arrow
  committer: Signature
  message: str
  parent_ids: List[str]
  tree_id: str
  hex_id: str

  @classmethod
  def from_pygit2(cls, commit: pg2.Commit) -> "Commit":
    return cls(
        author=Signature.from_pygit2(commit.author),
        commit_time=arrow.get(commit.commit_time),
        committer=Signature.from_pygit2(commit.committer),
        message=commit.message,
        parent_ids=[oid.hex for oid in commit.parent_ids],
        tree_id=commit.tree_id.hex,
        hex_id=commit.hex,
    )


@attr.s(frozen=True, slots=True, auto_attribs=True, init=True)
class Repo(object):
  pg2repo: pg2.Repository = attr.ib()

  @classmethod
  def open(cls, path: str) -> "Repo":
    return cls(pg2repo=pg2.Repository(path))

  @classmethod
  def wrap(cls, repo: pg2.Repository) -> "Repo":
    return cls(pg2repo=repo)

  @property
  def path(self) -> str:
    return self.pg2repo.path

  def commit(self, oid: OIDish) -> Commit:
    c = self.pg2repo.get(oid).peel(pg2.GIT_OBJ_COMMIT)
    return Commit.from_pygit2(c)

  def merge_base(self, a: OIDish, b: OIDish) -> Optional[Commit]:
    oid = self.pg2repo.merge_base(a, b)
    if oid is not None:
      return self.commit(oid.hex)
    else:
      return None

  def str_to_ref(self, s: str) -> Tuple[Ref, Commit]:
    r = self.pg2repo.lookup_reference(s)
    commit = self.commit(r.target)

    id_hex: str
    t = r.target
    if r.type == pg2.GIT_REF_OID:
      id_hex = cast(pg2.Oid, t).hex
    else:
      t = r.resolve().target
      id_hex = cast(pg2.Oid, t).hex

    return (
        Ref(name=r.name, id_hex=id_hex, author_email=commit.author.email),
        commit,
    )

  def iter_ref_and_commit(self) -> Iterable[Tuple[Ref, Commit]]:
    for r in self.pg2repo.references:
      yield self.str_to_ref(r)
