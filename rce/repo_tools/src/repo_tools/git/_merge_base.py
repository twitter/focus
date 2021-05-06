
from typing import Union
from .cmd import iterout
from .refs import Ref
from .consts import REFS_HEADS_MASTER

Refish = Union[Ref, str]

def find_merge_base(git_dir: str, ref: Refish, other: Refish = REFS_HEADS_MASTER) -> str:
  a = b = ""

  a = ref.name if isinstance(ref, Ref) else ref
  b = other.name if isinstance(other, Ref) else other

  return next(iterout(git_dir, ["merge-base", a, b]))
