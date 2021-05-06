# mypy: allow-any-expr

import re
import sys
import io
import os
import signal
from subprocess import CalledProcessError, DEVNULL, PIPE, Popen, TimeoutExpired, run
from contextlib import contextmanager
from typing import Any, IO, Iterator, List, Optional, Text, Tuple, Union
import attr

from loguru import logger

from .consts import REFS, REFS_HEADS, REFS_REMOTES, REFS_TAGS

Fileish = Union[int, None, IO[Any]]  # type: ignore


def _try_wait(popen: Popen[str], timeout: Optional[float] = None) -> bool:
  try:
    popen.wait(timeout)
    return True
  except TimeoutExpired:
    return False


def _signal(popen: Popen[str], sig: int) -> bool:
  try:
    os.kill(popen.pid, sig)
    return True
  except ProcessLookupError:
    return False


@contextmanager
def safe_wait(popen: Popen[str],
              initial_wait: float = 10.0,
              term_wait: Optional[float] = None) -> Iterator[None]:
  try:
    yield
  finally:
    if popen.stdout is not None and not popen.stdout.closed:
      popen.stdout.close()

    if _try_wait(popen, initial_wait):
      return

    if not _signal(popen, signal.SIGTERM):
      return

    if _try_wait(popen, term_wait):
      return

    if not _signal(popen, signal.SIGKILL):
      return

    if _try_wait(popen, term_wait):
      return

    raise RuntimeError(f"failed to kill process {popen.pid}")


def iterout(git_dir: str, args: List[str], exit_timeout: float = 15) -> Iterator[str]:
  if args[0] == "git":
    raise ValueError(
        "you should not specify 'git' as the first argument, only the subcommand and its arguments"
    )

  cmd = ["git", "-C", git_dir, *args]

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

  with safe_wait(po, exit_timeout):
    sout = po.stdout
    assert sout is not None

    while True:
      line = sout.readline()
      if line:
        yield line.strip()
      else:
        break

  assert po.returncode is not None
  if po.returncode != 0:
    raise CalledProcessError(po.returncode, cmd)
