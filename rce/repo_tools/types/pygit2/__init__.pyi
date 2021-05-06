from typing import Any, Iterator, List, Optional, Tuple, Union

features: Any
GIT_FEATURE_THREADS: int
GIT_FEATURE_HTTPS: int
GIT_FEATURE_SSH: int
GIT_REPOSITORY_INIT_OPTIONS_VERSION: int
GIT_REPOSITORY_INIT_BARE: int
GIT_REPOSITORY_INIT_NO_REINIT: int
GIT_REPOSITORY_INIT_NO_DOTGIT_DIR: int
GIT_REPOSITORY_INIT_MKDIR: int
GIT_REPOSITORY_INIT_MKPATH: int
GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE: int
GIT_REPOSITORY_INIT_RELATIVE_GITLINK: int
GIT_REPOSITORY_INIT_SHARED_UMASK: int
GIT_REPOSITORY_INIT_SHARED_GROUP: int
GIT_REPOSITORY_INIT_SHARED_ALL: int
GIT_REPOSITORY_OPEN_NO_SEARCH: int
GIT_REPOSITORY_OPEN_CROSS_FS: int
GIT_REPOSITORY_OPEN_BARE: int
GIT_REPOSITORY_OPEN_NO_DOTGIT: int
GIT_REPOSITORY_OPEN_FROM_ENV: int
GIT_ATTR_CHECK_FILE_THEN_INDEX: int
GIT_ATTR_CHECK_INDEX_THEN_FILE: int
GIT_ATTR_CHECK_INDEX_ONLY: int
GIT_ATTR_CHECK_NO_SYSTEM: int
GIT_FETCH_PRUNE_UNSPECIFIED: int
GIT_FETCH_PRUNE: int
GIT_FETCH_NO_PRUNE: int
GIT_OBJ_COMMIT: int
GIT_OBJ_TREE: int
GIT_OBJ_BLOB: int
GIT_OBJ_TAG: int
GIT_REF_OID: int
GIT_REF_SYMBOLIC: int
LIBGIT2_VER: Any

GitConstT = int


class Oid:
  hex: str
  raw: bytes


OIDish = Union[Oid, str]


class Repository:
  path: str
  references: References

  def __init__(
      self,
      path: Optional[str] = ...,
      flags: Optional[int] = ...,
  ) -> None:
    ...

  def lookup_reference_dwim(self, name: str) -> Reference:
    ...

  def lookup_reference(self, name: str) -> Reference:
    ...

  def resolve_refish(self, refish: str) -> Tuple[Commit, Reference]:
    ...

  def get(self, oid: OIDish) -> Object:
    ...

  def free(self) -> None:
    ...

  def merge_base(self, a: OIDish, b: OIDish) -> Optional[Oid]:
    ...


class References:

  def __iter__(self) -> Iterator[str]:
    ...

  def __getitem__(self, name: str) -> Reference:
    ...

  def __contains__(self, name: str) -> bool:
    ...

  def delete(self, name: str) -> None:
    ...

  def compress(self) -> None:
    ...

  def get(self, key: str) -> Reference:
    ...


class Object:
  hex: str
  type: GitConstT
  type_str: str

  def peel(self, type: Optional[GitConstT] = ...) -> Commit:
    ...


class Commit(Object):
  author: Signature
  committer: Signature
  commit_time: int
  message: str
  parent_ids: List[Oid]
  tree_id: Oid


class Signature:
  email: str
  name: str
  offset: int
  raw_email: bytes
  raw_name: bytes
  time: int


OidOrString = Union[Oid, str]
OidOrBytes = Union[Oid, bytes]


class Reference:
  name: str
  shorthand: str
  type: int
  target: OidOrString
  raw_target: OidOrBytes
  raw_shorthand: bytes
  raw_name: bytes

  def resolve(self) -> "Reference":
    ...

  def __eq__(self, value: Any) -> bool:
    ...

  def __ne__(self, value: Any) -> bool:
    ...

  def delete(self) -> None:
    ...

  def rename(self, name: str) -> None:
    ...

  def peel(self, type: Optional[GitConstT] = ...) -> Commit:
    ...
