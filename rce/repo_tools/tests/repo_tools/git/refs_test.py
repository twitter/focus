# mypy: allow-any-expr
from repo_tools.git import Ref
import pytest


@pytest.fixture
def valid_ref() -> Ref:
  return Ref(
      name="refs/heads/billy/feature/xyz123",
      id_hex="0123456789012345678901234567890123456789",
      author_email="billy@twitter.com",
  )


def test_ref_lstrip(valid_ref: Ref) -> None:
  assert valid_ref.lstrip(2) == "billy/feature/xyz123"


def test_valid_ref_behavior(valid_ref: Ref) -> None:
  assert not valid_ref.is_tag
  assert not valid_ref.is_remote
  assert valid_ref.is_branch
  assert valid_ref.namespace is None
  assert valid_ref.has_valid_twitter_email
  assert valid_ref.twitter_email_local == "billy"
  assert valid_ref.nth(2) == "billy"
