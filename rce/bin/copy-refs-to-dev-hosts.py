#!/usr/bin/env python
from shlex import split as shplit
import subprocess
from collections import defaultdict
import sys

def debug(msg):
  print(msg, file=sys.stderr)

def get_users():
  p = subprocess.run(
    shplit("git for-each-ref --format='%(authoremail:localpart)' refs/heads"),
    capture_output=True,
    check=True,
    encoding='utf8'
  )
  users = [line.strip() for line in p.stdout.splitlines() if line != ""]
  return sorted(list(set(users)))

def get_user_refs_map():
  p = subprocess.run(
    shplit("git for-each-ref --format='%(authoremail:localpart)%09%(refname:short)' refs/heads"),
    capture_output=True,
    check=True,
    encoding='utf8'
  )

  d = defaultdict(list)
  for line in p.stdout.splitlines():
    user, ref = line.strip().split("\t", 1)
    d[user].append(ref)

  return d


def main():
  user_refs = get_user_refs_map()

  users = sorted(user_refs.keys())

  for user in users:
    refspecs = []
    for ref in user_refs[user]:
      if ref.startswith(f"{user}/"):
        refspecs.append(f"refs/heads/{ref}:refs/heads/{ref}")
      else:
        refspecs.append(f"refs/heads/{ref}:refs/heads/{user}/{ref}")

    twgit_url = f"twgit://source/dev/{user}"
    cmd = ['git', 'push', twgit_url, *refspecs]

    debug(f"running: {' '.join(cmd)}")
    subprocess.run(cmd, check=False) # XXX: this hsould be True


if __name__ == '__main__':
  main()
