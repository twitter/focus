# Usage

## Create a new sparse repo

A `focus`-backed sparse repository is created from a dense repository. They're meant to be lightweight, so you can create multiple as necessary, e.g. one per feature.

Before trying to clone a repo, make sure that it's set up for `focus` (see [Administration](#administration)).

To create a new `focus` repository named `smallrepo` from your existing repo `~/monorepo`, run

```sh
$ focus new --dense-repo ~/monorepo smallrepo
```

which will create the `smallrepo` repository in the current directory.

## Add targets

There are two kinds of targets:

- Bazel targets, indicated by the scheme `bazel:`.
  - Adding a Bazel target to your repo will update the sparse checkout to include that target's files *and* all of its dependency packages.
  - Example: `bazel:path/to/package:target`.
  - You can also use `/...` to indicate "all subpackges of this package".
  - Example: `bazel:path/to/package/...`.
- Directory targets, indicated by the scheme `directory:`.
  - Adding a directory target to your repo will force that directory to be included in the sparse checkout. This is useful for directories which aren't Bazel packages.

To check out a target, run `focus add` inside the sparse repo:

```sh
$ focus add bazel:path/to/package:target
```

This updates the sparse checkout with `path/to/package:target` and all of its dependencies.

## Add projects

The members of your repository may have set up predefined projects to check out. A "project" is a named collection of Bazel and directory targets. If you know the name of the project you want to add, you can add it with `focus add`:

```sh
$ focus add my-project
```

Otherwise, you can browse and select projects with the interactive project selector:

```sh
$ focus add -i
```
