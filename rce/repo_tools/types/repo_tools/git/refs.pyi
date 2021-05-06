from _typeshed import SupportsLessThan, SupportsLessThanT
from typing import Iterator, List, Optional, Tuple


def for_each(
    git_dir: str,
    pattern: Optional[str] = ...,
    format: Optional[str] = ...,
    sort_by: Optional[str] = ...,
) -> List[str]:
  ...


def pack(git_dir: str) -> None:
  ...


def all(git_dir: str) -> List[str]:
  ...


def iter(
    git_dir: str,
    pattern: Optional[str] = ...,
    format: Optional[str] = ...,
    sort_by: Optional[str] = ...,
) -> Iterator[str]:
  ...


def iter_with_id(
    git_dir: str,
    pattern: Optional[str] = ...,
    sort_by: Optional[str] = ...,
) -> Iterator[Tuple[str, str]]:
  ...


class InvalidTwitterEmailUserError(Exception):
  ref: Ref
  message: str


class Ref(SupportsLessThan):
  name: str
  id_hex: str
  author_email: str

  def __init__(
      self,
      name: str,
      id_hex: str,
      author_email: str,
  ) -> None:
    ...

  def startswith(self, pfx: str) -> bool:
    ...

  @property
  def twitter_email_local(self) -> str:
    ...

  @property
  def is_tag(self) -> bool:
    ...

  @property
  def is_branch(self) -> bool:
    ...

  @property
  def is_remote(self) -> bool:
    ...

  @property
  def namespace(self) -> Optional[str]:
    ...

  @property
  def has_valid_twitter_email(self) -> bool:
    ...

  def lstrip(self, n: int) -> Optional[str]:
    ...

  def nth(self, n: int) -> str:
    ...


def iter_refs(
    git_dir: str,
    pattern: Optional[str] = ...,
    sort_by: Optional[str] = ...,
) -> Iterator[Ref]:
  ...
