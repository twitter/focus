#!/usr/bin/python3

import os
import pprint
import re
import sys
import subprocess
import math

HAS_USER_RE = re.compile(
    r"""
    ^/([^/]+) # the repository part of the path 'source.git'
    /         #
    ([^/]+)   # a potential username or keyword (tbd)
    /((?:git-(?:upload|receive)-pack)|info/refs)$ # the git command to run
    """,
    re.VERBOSE)

# we have to call the wrapper so our config overrides are picked up :P
GIT_BIN = os.environ.get("GIT_BIN", "/usr/bin/git")

# these are CGI env vars that we need to inspect and manipulate
PATH_INFO = os.environ['PATH_INFO']
PATH_TRANSLATED = os.environ['PATH_TRANSLATED']
REQUEST_URI = os.environ['REQUEST_URI']

# The name of the config key we use to selectiely hide and show refs to users
XFR_HIDE_REFS = "transfer.hideRefs"

def main():
  env = {}
  # we have to call the 'git' wrapper binary so that it adds the
  # overriding '-c' arguments in terms of config. this has the highest
  # precedence of all of the various sources of configuration.
  git_args = ['git']

  git_args.extend([
    # hide tags by default
    '-c', f"{XFR_HIDE_REFS}=refs/tags",
    # hide all refs by default, which we'll override with following rules
    '-c', f"{XFR_HIDE_REFS}=refs/heads",
    # unhide master, because people usually want it
    '-c', f"{XFR_HIDE_REFS}=!refs/heads/master",
  ])

  m = HAS_USER_RE.match(PATH_INFO)
  if m is not None:
    print(f"found match in {PATH_INFO}", file=sys.stderr)
    user = m.group(2)

    # if a person uses the url https://git.twitter.biz/foo.git/_all
    # we turn off all the ref hiding and expose everything to them,
    # for example if they want to use a custom refspec.
    # we use a leading underscore in case we hire someone whose LDAP username
    # works out to be 'all' (remember Edward Ng?)
    if user == '_all':
      git_args.extend([
        '-c', f"{XFR_HIDE_REFS}=!refs",  # show everything!
      ])
    # if the user wants to see just the tags, i.e. for follow-tags or
    # some such, then they can use https://git.twitter.biz/foo.git/_tags
    elif user == '_tags':
      git_args.extend([
        '-c', f"{XFR_HIDE_REFS}=!refs/tags",  # show everything!
      ])
    else:
      # in the usual case, expose the user's own refs to them
      git_args.extend([
        '-c', f"{XFR_HIDE_REFS}=!refs/heads/{user}",
      ])

    # now we have to clean up these variables to remove our extra path junk
    # so that git-http-backend knows which repository we're referring to
    env["PATH_INFO"] = PATH_INFO.replace(f"{user}/", "")
    env['PATH_TRANSLATED'] = PATH_TRANSLATED.replace(f"{user}/", "")
    env['REQUEST_URI'] = REQUEST_URI.replace(f"{user}/", "")

  # create a process environment based on our own, replacing
  # specific vars with the contents of 'env'
  child_env = dict(os.environ)
  child_env.update(env)

  # add the name of the program to invoke after all the flags we pass to
  # the 'git' binary
  git_args.append("http-backend")

  print("\nchild environment", file=sys.stderr)
  pprint.pprint(child_env, stream=sys.stderr)
  print(f"git args: {git_args!r}", file=sys.stderr)

  # os.exec* doesn't flush file descriptors so se do that here in case
  # there's anything waiting in a python controlled buffer
  sys.stdout.flush()
  sys.stderr.flush()

  # goodbye cruel world!
  os.execve(GIT_BIN, git_args, child_env)

if __name__ == '__main__':
  main()
