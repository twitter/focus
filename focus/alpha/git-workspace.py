#!/usr/bin/env python2.7

#
# git-workspace
# Create an additional working directory attached to a source repository.
# See `git workspace -h` for usage.
#

import argparse
import logging
import os
import shutil
import subprocess
import sys

from contextlib import contextmanager


@contextmanager
def git_repo_lock(path):
  lock = None
  try:
    lock = os.open(path, os.O_CREAT | os.O_WRONLY | os.O_EXCL)
    print_diagnostic("Acquired {}".format(path))
    yield lock
  finally:
    if lock is None:
      die("Could not acquire lock: {}".format(path))
    os.close(lock)
    os.remove(path)
    print_diagnostic("Released {}".format(path))


def die(text):
  """
  Print text to stderr and terminate execution, passing -1 to the invoking
  shell.

  :param text: The error message to be printed to stderr.
  """
  logging.critical(text)
  sys.exit(-1)


def print_diagnostic(text):
  """
  Print diagnostic text to stderr.

  :param text: The diagnostic to be printed to stderr.
  """
  logging.info(text)


class GitCommand(object):
  """
  An abstraction to run git commands in a specific repository. Each method
  runs one git command and returns its output as a byte string to the caller.
  """

  def __init__(self, path):
    self._path = path

  def version(self):
    return self._run_git_command(["--version"], workdir=False)

  def init(self):
    return self._run_git_command(["init"])

  def checkout(self, branch, treeish):
    # checkout uses check_call instead of check_output in order to be able
    # to display git's stderr in real time.
    if treeish:
      return self._run_git_command(["checkout", "-b", branch, treeish], call=True)
    else:
      return self._run_git_command(["checkout", branch], call=True)

  def sparse_checkout_init(self):
    # Run `git sparse-checkout init --cone`
    self._run_git_command(["sparse-checkout", "init", "--cone"])
    return self._run_git_command(["sparse-checkout", "add", ".git/info/sparse-checkout"])

  def config(self, setting, value):
    return self._run_git_command(['config', setting, value], call=True)

  def rev_parse(self, symbol):
    return self._run_git_command(["rev-parse", symbol])

  def _run_git_command(self, argv_, workdir=True, call=False):
    argv = ["git"]
    if workdir:
      argv.append("-C")
      argv.append(self._path)
    argv.extend(argv_)

    print_diagnostic("GitCommand " + str(argv))

    if call:
      result = subprocess.check_call(argv, stderr=subprocess.STDOUT)
    else:
      result = subprocess.check_output(argv, stderr=subprocess.STDOUT)
      result = result.strip()

    print_diagnostic("GitCommand " + str(result))

    return result


class GitRepoPath(object):
  """
  An abstraction to help simplify working with paths to different interesting
  directories within a git repository.
  """

  def __init__(self, path):
    self._path = os.path.abspath(path)
    self._dotgit = os.path.join(self._path, ".git")
    self._config = os.path.join(self._dotgit, "config")
    self._head = os.path.join(self._dotgit, "HEAD")
    self._hooks = os.path.join(self._dotgit, "hooks")
    self._hooksmulti = os.path.join(self._dotgit, "hooks_multi")
    self._info = os.path.join(self._dotgit, "info")
    self._objects = os.path.join(self._dotgit, "objects")
    self._alternates = os.path.join(self._objects, "info", "alternates")
    self._journals = os.path.join(self._objects, "journals")
    # todo(kaushik): SRC-2354: Generalize this for multiple remotes.
    self._statebinlock = os.path.join(self._journals, "origin", "state.bin.lock")
    self._packedrefs = os.path.join(self._dotgit, "packed-refs")
    self._prunedodb = os.path.join(self._dotgit, "pruned-odb", "objects")
    self._refs = os.path.join(self._dotgit, "refs")
    self._repod = os.path.join(self._dotgit, "repo.d")
    self._sparse = os.path.join(self._dotgit, "info", "sparse-checkout")

  def root(self): return self._path
  def dotgit(self): return self._dotgit
  def config(self): return self._config
  def head(self): return self._head
  def hooks(self): return self._hooks
  def hooksmulti(self): return self._hooksmulti
  def info(self): return self._info
  def objects(self): return self._objects
  def alternates(self): return self._alternates
  def journals(self): return self._journals
  def statebinlock(self): return self._statebinlock
  def packedrefs(self): return self._packedrefs
  def prunedodb(self): return self._prunedodb
  def refs(self): return self._refs
  def repod(self): return self._repod
  def sparse(self): return self._sparse

  def paths(self):
    """
    Yield absolute paths to various files / directories within the git
    repository rooted at `self._path`.
    """
    for path in self.__dict__.values():
      # todo(kaushik): SRC-2354: Fix ugly hack.
      filename = os.path.basename(path)
      if filename not in ("state.bin.lock", "sparse-checkout"):
        yield path


def parse_argv(argv):
  """
  Parse argv and return a dictionary of configuration options.

  :param argv: A list of arguments.
  :returns:    A dictionary of configuration options.
  """
  parser = argparse.ArgumentParser(description="git-workspace: Create an additional working directory attached to a source repository.")
  parser.add_argument("source_path", help="Path to the source directory the workspace should be derived from.")
  parser.add_argument("workspace_path", help="Path to the (as yet non existent) directory the workspace should be created in.")
  parser.add_argument("--branch", default="master", help="To checkout an existing branch specify BRANCH (This is the equivalent of running `git checkout BRANCH` in the workspace). To create a new branch specify BRANCHNAME:TREEISH (This is the equivalent of running `git checkout -b BRANCHNAME TREEISH` in the workspace).", action="store")
  parser.add_argument("--skip-lock-source", default=False,
                      help="Do not lock source repo while creating a workspace - Useful if `source_path` is on a read-only volume", action="store_true")
  parser.add_argument("--sparse", help="To perform a sparse checkout specify SPARSE with the sparse specification", action="store")

  config = parser.parse_args(argv)
  config.trace = False
  debug = os.environ.get("GIT_TRACE")
  if debug is not None:
    if (debug is "1") or (debug.upper() == "TRUE"):
      config.trace = True

  return config


def check_source_repository_integrity(s_path):
  """
  Check if the repository at s_path.root() is a source repository.

  :param s_path: An instance of GitPath pointing to the source repository.
  """
  # This isn't bullet proof but is a pretty good approximation of a check for
  # whether or not the given repository is source.

  # Check if all the paths we care about exist in the source repository.
  for path in s_path.paths():
    if not os.path.exists(path):
      die("Missing path in source repository: " + path + ". For non source repositories use `git-worktree`.")

  # Check remote URL in config
  config = None
  with open(s_path.config(), "r") as config_file:
    config = config_file.read()

  if not ("https://git.twitter.biz/source" in config or "https://git.twitter.biz/ro/source" in config):
    die("Invalid source repository configuration. For non source repositories use `git-worktree.`")


def create_workspace(s_path, s_gitcmd, w_path, w_gitcmd, skip_lock_source):
  """
  Given a GitCommand and GitPath instance for the source repository and the
  workspace (to be created), create and initialize the workspace.

  :param s_path: An instance of GitPath for the source repository.
  :param s_gitcmd: An instance of GitCommand for the source repository.
  :param w_path: An instance of GitPath for the workspace.
  :param w_gitcmd: An instance of GitCommand for the workspace.
  :param skip_lock_source: Do not create any lock files in source repository.
  """
  # Create the workspace directory.
  print_diagnostic("Creating workspace " + w_path.root())
  os.makedirs(w_path.root())

  # Create the workspace repo.
  print_diagnostic("Running `git init` in " + w_path.root())
  w_gitcmd.init()

  # Copy HEAD, config, packed-refs.
  print_diagnostic("Setting up HEAD, config and packed-refs for " + w_path.root())
  shutil.copy(s_path.head(), w_path.head())
  shutil.copy(s_path.config(), w_path.config())
  shutil.copy(s_path.packedrefs(), w_path.packedrefs())

  # Copy hooks.
  print_diagnostic("Copying hooks into " + w_path.dotgit())
  shutil.rmtree(w_path.hooks(), ignore_errors=True)
  shutil.copytree(s_path.hooks(), w_path.hooks())
  shutil.copytree(s_path.hooksmulti(), w_path.hooksmulti())

  # Copy info.
  print_diagnostic("Copying info into " + w_path.dotgit())
  shutil.rmtree(w_path.info(), ignore_errors=True)
  shutil.copytree(s_path.info(), w_path.info())

  # Copy refs.
  print_diagnostic("Copying refs into " + w_path.dotgit())
  shutil.rmtree(w_path.refs(), ignore_errors=True)
  shutil.copytree(s_path.refs(), w_path.refs())

  # Copy repo.d.
  print_diagnostic("Copying repo.d into " + w_path.dotgit())
  shutil.copytree(s_path.repod(), w_path.repod())

  # Create the alternates file for the workspace.
  print_diagnostic("Writing " + w_path.alternates())
  with open(w_path.alternates(), "w") as alternates:
    alternates.write(s_path.prunedodb() + "\n")
    alternates.write(s_path.objects() + "\n")
    alternates.write(w_path.prunedodb() + "\n")

  # Rewrite the alternates file in the source directory to contain only
  # absolute paths.
  # Note: Can't use os.path.join here because we *do* want the literal
  # concatenation of the ".original" suffix to the alternates file path.
  if not skip_lock_source:
    # Let's assume that since the user does not want locks in source repository, the user
    # would also not want any files created/changed in the source repository.
    shutil.move(s_path.alternates(), s_path.alternates() + ".original")
    with open(s_path.alternates(), "w") as alternates:
      alternates.write(s_path.prunedodb() + "\n")

  # Make the pruned-odb/objects folder in the workspace repo.
  print_diagnostic("Creating " + w_path.prunedodb())
  os.makedirs(w_path.prunedodb())

  # Setup journal state.
  print_diagnostic("Copying journal state")
  shutil.copytree(s_path.journals(), w_path.journals())
  # Delete the lock file in the workspace.
  if not skip_lock_source:
    os.remove(w_path.statebinlock())


def setup_logging(config):
  if config.trace:
    level = logging.INFO
  else:
    level = logging.CRITICAL
  logging.basicConfig(level=level,
                      format="git-workspace: %(levelname)-8s %(message)s")


def main(argv):
  """
  Create a git workspace.

  :param argv: A list of command line options.
  """
  config = parse_argv(argv)
  setup_logging(config)

  s_path = GitRepoPath(config.source_path)
  s_gitcmd = GitCommand(s_path.root())
  w_path = GitRepoPath(config.workspace_path)
  w_gitcmd = GitCommand(w_path.root())

  # Check if the twitter git client is in PATH. We'll need it to update the workspace later.
  output = s_gitcmd.version()
  if not "twtr" in output:
    die("Twitter git client not in PATH.")

  # Check if we've been given a valid source repository to copy from.
  check_source_repository_integrity(s_path)

  # Make sure the workspace directory doesn't already exist.
  if os.path.exists(w_path.root()):
    die("Workspace directory already exists: " + w_path.root())

  # Check if the branch name (if specified) is valid.
  branch_name = config.branch
  treeish = None
  if ":" in config.branch:
    tokens = config.branch.split(":")
    if len(tokens) != 2:
      die("Invalid branch specification " + config.branch)

    branch_name = tokens[0]
    treeish = tokens[1]
    try:
      s_gitcmd.rev_parse(treeish)
    except Exception as e:
      die("Invalid treeish " + treeish + ". Use --branch NAME to checkout an existing branch. Original error: " + str(e))
  else:
    try:
      s_gitcmd.rev_parse(branch_name)
    except Exception as e:
      die("Branch " + branch_name + " does not exist. Use --branch NAME:TREEISH to create a new branch. Original error: " + str(e))

  # Unless, skip_lock_source is specific - acquire the state.bin.lock
  # to prevent concurrent updates to the directories create_workspace will attempt to copy.
  # TODO (SRC-4158) - Add intelligence here so we always have repo level locks
  if config.skip_lock_source:
    create_workspace(s_path, s_gitcmd, w_path, w_gitcmd, config.skip_lock_source)
  else:
    with git_repo_lock(s_path.statebinlock()):
      create_workspace(s_path, s_gitcmd, w_path, w_gitcmd, config.skip_lock_source)

  # If sparse checkout is enabled, set it up before the checkout
  if config.sparse:
    print_diagnostic("Enabling sparse checkout")
    w_gitcmd.config('core.sparseCheckout', 'true')
    shutil.copy(config.sparse, w_path.sparse())
    w_gitcmd.sparse_checkout_init()

  # Create the working directory
  w_gitcmd.checkout(branch_name, treeish)


if __name__ == '__main__':
  main(sys.argv[1:])
