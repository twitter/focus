# mypy: allow-any-expr
import os
import sys
from subprocess import CalledProcessError, PIPE, Popen
from typing import List, Set, Tuple

from loguru import logger

from . import git
from .git import Ref, refs
from .git.consts import REFS_HEADS, REFS_HEADS_SLASH, REFS_NS_P


def get_usernames(git_dir: str) -> List[str]:
  logger.info("getting list of usernames")
  s: Set[str] = set()
  ref: Ref
  for ref in refs.iter_refs(git_dir, pattern=REFS_HEADS):
    if ref.startswith(REFS_HEADS_SLASH):
      stripped = ref.name[len(REFS_HEADS_SLASH):]
      i = stripped.index("/")
      assert i > 0
      s.add(stripped[0:i])
  return sorted(list(s))


def hash_users(users: List[str]) -> List[Tuple[str, str]]:
  logger.info("hashing usernames")
  input = '\n'.join(users + [""])

  cmd = ["twgit", "hash", "--repo=source", "--role=dev", "-"]
  po = Popen(cmd, encoding='utf8', stdin=PIPE, stdout=PIPE, stderr=sys.stderr)

  stdout, _ = po.communicate(input=input)
  po.wait()
  assert po.returncode is not None
  if po.returncode != 0:
    raise CalledProcessError(po.returncode, cmd)

  return [(m[0], m[1]) for m in (n.split("\t", maxsplit=1) for n in stdout.splitlines())]


def user_to_ns_mapping(git_dir: str) -> List[Tuple[str, str]]:
  return hash_users(get_usernames(git_dir))


def namespace_devs(git_dir: str) -> None:
  unsd = {user: ns for user, ns in user_to_ns_mapping(git_dir)}

  with git.UpdateRef() as ur:
    for ref in refs.iter_refs(git_dir, pattern=REFS_HEADS):
      simple = ref.lstrip(2)  # refs/heads/foo/bar -> foo/bar
      if simple is None:
        continue
      ns = unsd[ref.nth(2)]  # usnd["foo"] -> dev00

      ns_ref = REFS_NS_P.joinpath(
          ns, REFS_HEADS, simple
      )  # refs/namespaces/dev00/refs/heads/foo/bar
      ur.create(str(ns_ref), ref.id_hex)
      ur.delete(ref.name)
    ur.run(git_dir)


def main() -> None:
  git_dir = os.environ.get("REPO_TOOLS_GIT_DIR", "/repos/source/scratch/full.git")
  namespace_devs(git_dir)
  refs.pack(git_dir)


if __name__ == "__main__":
  main()
