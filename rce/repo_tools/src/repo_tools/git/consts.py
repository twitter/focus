from pathlib import PurePosixPath

REFS = "refs"
REFS_P = PurePosixPath("refs")

REFS_HEADS = f"{REFS}/heads"
REFS_HEADS_P = PurePosixPath(REFS_HEADS)

REFS_HEADS_SLASH = f"{REFS_HEADS}/"

REFS_REMOTES = "refs/remotes"
REFS_REMOTES_P = PurePosixPath(REFS_REMOTES)

REFS_REMOTES_ORIGIN = "refs/remotes/origin"
REFS_REMOTES_ORIGIN_P = PurePosixPath(REFS_REMOTES_ORIGIN)
REFS_REMOTES_ORIGIN_SLASH = f"{REFS_REMOTES_ORIGIN}/"

REFS_HEADS_MASTER = "refs/heads/master"

REFS_TAGS = "refs/tags"
REFS_TAGS_P = PurePosixPath(REFS_TAGS)

REFS_NS = "refs/namespaces"
REFS_NS_P = PurePosixPath(REFS_NS)

__all__ = (
    "REFS",
    "REFS_P",
    "REFS_HEADS",
    "REFS_HEADS_P",
    "REFS_HEADS_SLASH",
    "REFS_REMOTES",
    "REFS_REMOTES_P",
    "REFS_REMOTES_ORIGIN",
    "REFS_REMOTES_ORIGIN_P",
    "REFS_REMOTES_ORIGIN_SLASH",
    "REFS_HEADS_MASTER",
    "REFS_TAGS",
    "REFS_TAGS_P",
    "REFS_NS",
    "REFS_NS_P",
)
