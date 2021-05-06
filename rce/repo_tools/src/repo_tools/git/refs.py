# mypy: allow-any-expr
import re
import sys
import io
import os
from subprocess import CalledProcessError, DEVNULL, PIPE, Popen, run
from typing import Any, IO, Iterator, List, Optional, Text, Tuple, Union
import attr

from loguru import logger

from .consts import REFS, REFS_HEADS, REFS_REMOTES, REFS_TAGS
import pygit2 as pg2

_NS_RE = re.compile(r'''refs/namespaces/([^/]+)''')
_AT_TWITTER_DOT_COM = '@twitter.com'


@attr.s(slots=True, frozen=True, auto_attribs=True, eq=True, order=True, init=True, kw_only=True)
class Ref(object):
  name: str
  id_hex: str
  # user is based on author email
  # this field will be None if the email is not @twitter.com
  author_email: str


  def startswith(self, pfx: str) -> bool:
    return self.name.startswith(pfx)

  @property
  def is_tag(self) -> bool:
    """returns True if this is a tag (lightweight or regular)"""
    return self.name.startswith(REFS_TAGS)

  @property
  def is_branch(self) -> bool:
    """returns True if this is a local branch (non-tag)"""
    return self.name.startswith(REFS_HEADS)

  @property
  def is_remote(self) -> bool:
    """returns True if this is a remote tracking branch"""
    return self.name.startswith(REFS_REMOTES)

  @property
  def namespace(self) -> Optional[str]:
    """returns the namespace according to the gitnamespaces(7) format

    if the ref is 'refs/namespaces/foo/refs/namespaces/bar' then this
    method will return "foo/bar".

    returns None if there is no namespace
    """
    s = '/'.join([m[1] for m in _NS_RE.finditer(self.name)])
    return s if s else None

  @property
  def has_valid_twitter_email(self) -> bool:
    return bool(self._email_local)

  @property
  def _email_local(self) -> Optional[str]:
    """return the 'local' part of the email address, i.e. the part to the left of the '@'
    if the email is a @twitter.com email, else None
    """
    if self.author_email.endswith(_AT_TWITTER_DOT_COM):
      m = self.author_email[0:len(self.author_email) - len(_AT_TWITTER_DOT_COM)]
      return m if m else None
    else:
      return None

  @property
  def twitter_email_local(self) -> str:
    """returns the local part of a twitter.com email address
    IF this commit does not have a valid twitter email address, raises
    InvalidTwitterEmailUserError. You can check with has_valid_twitter_email
    to avoid the exception
    """
    local = self._email_local
    if local is None:
      raise InvalidTwitterEmailUserError.for_ref(self)
    else:
      return local

  def _path_els(self) -> List[str]:
    return self.name.split("/")

  def lstrip(self, n: int) -> Optional[str]:
    """returns the ref name with the leftmost 'n' elements removed
    if name is 'refs/heads/foo/bar' then r.lstrip(2) returns 'foo/bar'
    if we're unable to remove 'n' components, then return None
    """
    return '/'.join(self._path_els()[n:])

  def nth(self, n: int) -> str:
    """returns the 0-indexed nth-most path element (from the left)"""
    return self._path_els()[n]


@attr.s(auto_exc=True, auto_attribs=True, slots=True, frozen=True)
class InvalidTwitterEmailUserError(Exception):
  ref: Ref
  message: str

  @classmethod
  def for_ref(cls, ref: Ref) -> "InvalidTwitterEmailUserError":
    return cls(
        ref,
        f"ref {ref.name} has a non-twitter email address or invalid format: {ref.author_email!r}"
    )


_TAB = r'%09'
_REFNAME_ATOM = r'%(refname)'
# git's format is annoying in that the prefix '*' doesn't do the right thing
# for non-tag refs, so you have to do this annoying conditional shit
_OBJECTTYPE_ATOM = r'%(if:equals=tag)%(objecttype)%(then)%(*objectname)%(else)%(objectname)%(end)'
# same thing here
_AUTHOR_ATOM = r'%(if:equals=tag)%(objecttype)%(then)%(*authoremail:trim)%(else)%(authoremail:trim)%(end)'

_REF_FORMAT_STR = _TAB.join([_REFNAME_ATOM, _OBJECTTYPE_ATOM, _AUTHOR_ATOM])


def pack(git_dir: str) -> None:
  logger.info("packing refs")
  run(["git", "-C", git_dir, "pack-refs", "--all"], check=True)


DEFAULT_FORMAT = "%(refname)"


def for_each(
    git_dir: str,
    pattern: Optional[str] = None,
    format: Optional[str] = None,
    sort_by: Optional[str] = None,
) -> List[str]:
  return list(iter(git_dir, pattern, format, sort_by))


def all(git_dir: str) -> List[str]:
  return for_each(git_dir, pattern='refs/', format=DEFAULT_FORMAT, sort_by='refname')


Fileish = Union[int, None, IO[Any]]  # type: ignore


def iter(
    git_dir: str,
    pattern: Optional[str] = None,
    format: Optional[str] = None,
    sort_by: Optional[str] = None,
) -> Iterator[str]:

  ptrn = REFS if pattern is None else pattern
  fmt = DEFAULT_FORMAT if format is None else format

  cmd = ["git", "-C", git_dir, "for-each-ref", f"--format={fmt}"]

  if sort_by is not None:
    cmd.append(f"--sort={sort_by}")

  if not ptrn.endswith('/'):
    ptrn = ptrn + '/'

  cmd.append(ptrn)

  stderr: Fileish = 2
  try:
    stderr = sys.stderr.fileno()
  except io.UnsupportedOperation as e:
    stderr = None

  po = Popen(
      cmd,
      encoding='utf8',
      stdin=DEVNULL,
      stdout=PIPE,
      stderr=stderr,
  )

  sout = po.stdout
  assert sout is not None

  while True:
    line = sout.readline()
    if line:
      yield line.strip()
    else:
      break

  po.wait(timeout=60)
  assert po.returncode is not None
  if po.returncode != 0:
    raise CalledProcessError(po.returncode, cmd)


def iter_with_id(
    git_dir: str,
    pattern: Optional[str] = None,
    sort_by: Optional[str] = None,
) -> Iterator[Tuple[str, str]]:
  """iterate the pattern refs as tuples of (ref, hexid)
  tags will be peeled to their commits.
  """
  # the * in %(*objectname) tells git to show the object a tag refers to
  # (i.e. the commit) rather than the tag object itself
  for line in iter(git_dir, pattern, format=r'%(refname)%09%(*objectname)', sort_by=sort_by):
    a, b = line.split("\t", 1)
    yield a, b


def iter_refs(
    git_dir: str,
    pattern: Optional[str] = None,
    sort_by: Optional[str] = None,
) -> Iterator[Ref]:
  for line in iter(git_dir, pattern, format=_REF_FORMAT_STR, sort_by=sort_by):
    try:
      ref, hex, email = line.split("\t", 3)
    except ValueError as e:
      raise ValueError(f"failed to split line: {line!r}")
    yield Ref(name=ref, id_hex=hex, author_email=email)
