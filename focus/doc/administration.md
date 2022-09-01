# Administration

To enable a repo for `focus`, you typically need to do some setup for that repository to ensure that core Bazel rules are materialized. To start, create the directory `focus` in your repository. You'll commit the following changes to this directory into your codebase.

## `.gitignore`

In your repository, add `/.focus` to your `.gitignore` patterns. `focus` creates and uses this directory to manage the repo state.

## Mandatory projects

You will most likely want to include some targets in all sparse checkouts for this repository. Create `focus/mandatory.projects.json` to define these projects. Example:

```json
{
    "projects": [
        {
            "name": "source_bazel_implicits",
            "description": "Implicit dependencies required by Bazel in the Source repo",
            "mandatory": true,
            "targets": [
                "bazel://scrooge/scrooge-generator/...",
                "bazel://scrooge/scrooge-linter/...",
                "bazel://strato/src/main/scala/com/twitter/strato/cmd/compiler:ql-and-client-compiler",
                "bazel://tools/implicit_deps/...",
                "bazel://tools/scripts/...",
                "bazel://tools/src/main/...",
                "directory:3rdparty",
                "directory:scrooge-internal",
                "directory:tools"
            ]
        },
    ]
}
```

You can have as many of these projects as you like. (The main reason to create multiple mandatory projects is just for intelligibility.)

## Projects

These are the predefined projects that your users can select from. Create files of the form `focus/projects/<project>.project.json` to configure them, using patterns in the same form as for [Mandatory layers](#mandatory-layers), except omitting the `"mandatory": true` item.

If you don't want to configure any projects right now, create an empty `.gitkeep` file to commit the otherwise empty directory to the repository.

In your code review tool or permission management system, you can give users access to this directory to manage their own project definitions.


## Outlining patterns

When `focus` needs to query Bazel for build graph information, it runs Bazel in an internal Git clone of the source repository. To avoid materializing lots of files, that repository can be configured to use a sparse checkout with a subset of files which are Bazel-relevant. This will improve performance for operations like `focus sync`, especially when `HEAD` changes.

The "outlining patterns" are patterns for files that will be materialized in `focus`'s internal clone. Create `focus/outlining.patterns.json` with these contents (as an example):

```json
{
    "patterns": [
        {
            "kind": "Directory",
            "pattern": {
                "precedence": 0,
                "path": "",
                "recursive": true
            }
        },
        {
            "kind": "Directory",
            "pattern": {
                "path": "focus",
                "recursive": true
            }
        },
        {
            "kind": "Verbatim",
            "pattern": {
                "fragment": "WORKSPACE"
            }
        },
        {
            "kind": "Verbatim",
            "pattern": {
                "fragment": "WORKSPACE.*"
            }
        },
        {
            "kind": "Verbatim",
            "pattern": {
                "fragment": "BUILD"
            }
        },
        {
            "kind": "Verbatim",
            "pattern": {
                "fragment": "BUILD.*"
            }
        },
        {
            "kind": "Verbatim",
            "pattern": {
                "fragment": "*.bzl"
            }
        }
    ]
}
```

This example includes Bazel-relevant files like `BUILD` and `.bzl` files. You may want to add more files as appropriate for your organization. For example, we also include `*.thrift` files in our outlining patterns since many Bazel targets are built from Thrift binding files.

## Project index

Querying Bazel can be expensive, so `focus` uses a distributed cache to store a precomputed index for many `focus` queries. Each index is generated for a single commit of your repository (but common key-value pairs are shared between indexes for efficiency). You can generate an index as part of a hook or continuous integration job and make it available to your users.

TODO: explain how to use `focus index generate`/`focus index push`/`focus index fetch` to create and distribute index artifacts.
TODO: explain how `focus pull` can be used by end users.
