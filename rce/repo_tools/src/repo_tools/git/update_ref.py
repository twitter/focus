import os
import re
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from subprocess import CalledProcessError
from tempfile import TemporaryFile
from types import TracebackType
from typing import IO, List, Optional, Type

NUL = b"\x00"
TWO_NULS = b"\x00\x00"

SHA1_RE = re.compile(r'''^[0-9a-f]{40}$''')

class InvalidObjectIdError(Exception):
  pass

class UpdateRef(object):

  def __init__(self, fp: Optional[IO[bytes]] = None):
    self._fp = fp
    self._started = self._finalized = False

  def _write(self, *stuff: bytes) -> None:
    for b in list(stuff):
      self.file.write(b)

  def update(self, ref: str, new_oid: str) -> None:
    self._write(b"update ", ref.encode("utf8"), NUL, new_oid.encode("utf8"), TWO_NULS)

  def delete(self, ref: str) -> None:
    self._write(b"delete ", ref.encode('utf8'), NUL, NUL)

  def create(self, ref: str, oid: str) -> None:
    if not SHA1_RE.match(oid):
      raise InvalidObjectIdError(f"invalid object id {oid!r} for ref {ref!r}")
    self._write(b"create ", ref.encode('utf8'), NUL, oid.encode('utf8'), NUL)

  def start(self) -> None:
    self._started = True
    self._write(b"start", NUL)

  def prepare(self) -> None:
    assert self._started
    self._write(b"prepare", NUL)

  def commit(self) -> None:
    assert self._started
    self._write(b"commit", NUL)

  def flush(self) -> None:
    self.file.flush()

  def rewind(self) -> None:
    self.file.seek(0, os.SEEK_SET)

  def close(self) -> None:
    if self._fp is not None:
      self._fp.close()
      self._fp = None
      self._finalized = self._started = False

  def __enter__(self) -> "UpdateRef":
    if self._fp is None:
      self._fp = TemporaryFile()
    self.start()
    return self

  def __exit__(self, e1: Type[BaseException], e2: BaseException, e3: TracebackType) -> None:
    sys.exc_info
    self.close()

  @property
  def file(self) -> IO[bytes]:
    assert self._fp is not None
    return self._fp

  def done(self) -> None:
    assert not self._finalized
    self._finalized = True
    if self._started:
      self.prepare()
      self.commit()
    self.flush()
    self.rewind()

  def run(self, git_dir: str) -> None:
    if not self._finalized:
      self.done()
    run_z(git_dir, self.file)


def run_z(git_dir: str, input: IO[bytes]) -> None:
  po = subprocess.Popen(
      ["git", "-C", git_dir, "update-ref", "--stdin", "-z"],
      stdin=input,
      stdout=sys.stderr,
      stderr=sys.stderr,
  )
  _ = po.communicate()
  po.wait()
  assert po.returncode is not None
  if po.returncode != 0:
    raise CalledProcessError(po.returncode, po.args)


def delete(git_dir: str, refs: List[str]) -> None:
  with UpdateRef() as ur:
    for ref in refs:
      ur.delete(ref)
    ur.run(git_dir)


REFLOG_DIRNAME = "logs"


def rename_reflog(git_dir: str, old: str, new: str) -> None:
  reflogs = Path(git_dir).joinpath(REFLOG_DIRNAME)
  assert reflogs.is_dir
  oldlog, newlog = reflogs.joinpath(old), reflogs.joinpath(new)
  newlog.parent.mkdir(parents=True, exist_ok=True)
  oldlog.rename(newlog)
