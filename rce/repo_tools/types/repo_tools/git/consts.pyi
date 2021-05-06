from pathlib import PurePosixPath
from typing import Generic, Protocol, TypeVar

REFS: str
REFS_P: PurePosixPath
REFS_HEADS: str
REFS_HEADS_P: PurePosixPath
REFS_HEADS_SLASH: str
REFS_REMOTES: str
REFS_REMOTES_P: PurePosixPath
REFS_REMOTES_ORIGIN: str
REFS_REMOTES_ORIGIN_P: PurePosixPath
REFS_REMOTES_ORIGIN_SLASH: str
REFS_HEADS_MASTER: str
REFS_NS: str
REFS_NS_P: PurePosixPath
REFS_TAGS: str
REFS_TAGS_P: PurePosixPath

T = TypeVar("T")


class Freeable(Protocol):

  def free(self) -> None:
    ...
