import os
import shutil
from contextlib import contextmanager
from pathlib import Path
from shutil import rmtree
from tempfile import TemporaryDirectory
from typing import ContextManager, Generator, Iterable, Tuple, Union

import attr


@attr.s(auto_exc=True)
class FDNameConflict(OSError):
  ref = attr.ib(type=str)
  filename = attr.ib(type=str)
  conflict_ref = attr.ib(type=str)


def _tmpdir_path() -> Path:
  return Path(TemporaryDirectory().name)

PathishT = Union[Path, str]

@attr.s()
class FakeRefDb(object):
  root: Path = attr.ib(factory=_tmpdir_path)

  @classmethod
  def open(cls) -> ContextManager["FakeRefDb"]:
    return _refdb_context_mgr()

  def cleanup(self) -> None:
    shutil.rmtree(self.root, ignore_errors=True)

  def add_ref(self, ref: PathishT, orig_ref: str) -> None:
    try:
      actual: Path = self.root.joinpath(ref)
      actual.parent.mkdir(parents=True, exist_ok=True)
      with actual.open('w') as fp:
        fp.write(f"{orig_ref}\n")
    except (FileExistsError, IsADirectoryError, NotADirectoryError) as e:
      fname: str = e.filename
      cr = Path(fname).relative_to(self.root)
      raise FDNameConflict(ref=str(ref), filename=fname, conflict_ref=str(cr))

  def remove_r(self, ref: Path) -> None:
    actual: Path = self.root.joinpath(ref)
    rmtree(actual)

  def walk(self) -> Iterable[Tuple[str, str]]:
    for p, _, files in os.walk(self.root):
      for file in files:
        orig_ref = self.root.joinpath(p, file).read_text(encoding='utf8').strip()
        yield orig_ref, str(Path(p).joinpath(file).relative_to(self.root))


@contextmanager # type: ignore
def _refdb_context_mgr() -> Generator[FakeRefDb, None, None]:
  with TemporaryDirectory() as tmp:
    yield FakeRefDb(root=Path(tmp))
