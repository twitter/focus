# Focused Development Tools

## Synopsis

The `focus` suite aims to make it easy to work with partial source trees. The technique involves tight integration of Git and our build tool, Bazel. The underlying mechanism in Git that supports this suite is called "sparse checkouts". Sparse checkouts let Git filter which files and directories are present in a working tree based on a list of patterns. When the Git working tree's contents are being updated on disk, Git places only those files and directories matching the pattern list on disk. In effect, Git is hiding files, although they are still present in the underlying data store.

One of the major hurdles of source control performance is the vastness of our repository in terms of files/directories on disk and objects in Git's object store. Focused development aims to cut these by providing smaller project-focused repositories that are smaller by all of these measures. 

Engineers typically work on a relatively small subset of the monorepo at any given time. Focused development allows source control users to specify a set of build system coordinates either ad-hoc or as pre-defined layers. After deciding which coordinates or layers are necessary to achieve a task, engineers can get a small, focused repository that performs better than a full working copy.


## Limitations of the Current Implementation

### Bazel-only

Only targets with correctly modeled dependencies in Bazel will work right now; that is, the package list returned in `bazel query` for transitive dependencies must be correct.


### Full local clone

As of right now, the `focus` suite needs a full local copy of the source repository for a few reasons:
- We need an execution environment to run build graph queries and have not yet built a remote service to achieve this
- To perform initial cloning from, and to act as an "alternate" object store to support journaled fetches, since the source repo expressly disallows fetching without journals

We would like to eventually solve both of these, but are currently actively working on the latter.


### Transitive closure is materialized

Because the build system needs to be able to compile everything from source for now, all transitive source files and files necessary for the computation of the build graph itself must be present.


## Terminology

**Dense repos** are repositories with all files materialized in the working tree, and all backing objects present in the Git object store.

**Sparse repos** are sparse clones that have a sparse checkout pattern present, and a reduced set of objects in the Git object store. As of right now, they cannot function without their dense "parent" repository present, though we expect this to not be the case in the future.

**Coordinates** are build system target patterns. They can include wildcard patterns, but should be simple "label expressions". For example, coordinates to get all of Finagle might look something like `//finagle/...`. 

**Layers** are named, pre-defined sets of coordinates. 

**Layer Sets** are sets of layers. They are checked into the source repository in the `focus/projects` directory, and are expressed in JSON. The look like this example:

### Example Layer Set
```
{
    "layers": [
        {
            "name": "timeline_team/random_timelines",
            "description": "Randomized timelines. Who wouldn't love them?",
            "coordinates": [
                "//timelinemixer/random/..."
            ]
        }
    ]
}
```

> Note: these are loaded from multiple files, but the namespace is flat, so the names in different files must be unique. As of right now, the names of these "layer set" files themselves are not interpreted in any particular way. The names should follow a pattern of `<team_name>/<project_name>`. It would be nice if we could keep teams' layer sets defined in one file for ease of reference, and to mitigate the proliferation of layer sets.

**Stacks** are first-in-last-out stacks of layers tracked in sparse clones. They allow additional layers to be pushed onto the stack to expand the scope of the sparse repo. This allows you to quickly add more projects, hack on them, and possibly pop them off of your stack later.


## Getting Focused

Decide which targets you need to work with, and either define those in layer sets that are checked into the repository, or simply specify them as coordinates when running the `focus clone` command. In the case of not checking in defined layers, they are added to an "ad hoc" layer set that is written into the sparse repo.


### Setting up a sparse repo

Here's an example of cloning a repo with the "finagle" layer in it.

```
wilhelm at tw-mbp-wilhelm in ~/workspace
$ focus clone --dense-repo ~/workspace/source --sparse-repo ~/workspace/finagle --branch master --layers finagle
[2021-09-16T18:04:36Z INFO  focus::sparse_repos] Generating sparse profile
[2021-09-16T18:04:36Z INFO  focus::sparse_repos] Skipping generation of project view
[2021-09-16T18:04:36Z INFO  focus::sparse_repos] Creating a template clone
[2021-09-16T18:04:38Z INFO  focus::sparse_repos] Dependency query ["//tools/implicit_deps:thrift-implicit-deps-impl", "//scrooge-internal/...", "//loglens/loglens-logging/...", "//finagle/..."] yielded 669 directories
[2021-09-16T18:04:38Z INFO  focus::sparse_repos] Finished generating sparse profile
[2021-09-16T18:04:42Z INFO  focus::sparse_repos] Finished creating a template clone
[2021-09-16T18:04:42Z INFO  focus::sparse_repos] Copying configuration
[2021-09-16T18:04:43Z INFO  focus::sparse_repos] Configuring visible paths
[2021-09-16T18:04:46Z INFO  focus::sparse_repos] Adding directories
[2021-09-16T18:04:53Z INFO  focus::sparse_repos] Checking out the working copy
[2021-09-16T18:04:54Z INFO  focus::sparse_repos] Moving the project view into place
[2021-09-16T18:04:54Z INFO  focus::tracker] Assigning new UUID 20a53edb-3af8-48d6-9538-58e3644ddb86 for repo at path /Users/wilhelm/workspace/finagle
[2021-09-16T18:04:54Z INFO  focus::sparse_repos] Pushing the selected layers

wilhelm at tw-mbp-wilhelm in ~/workspace
$ cd finagle

wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus selected-layers
0: finagle (Finagle) -> ["//finagle/..."]
```

### Discovering available layers

I can see what layers are defined in the repository by using the `focus available-layers` command. This command also works in the dense (full copy of source) repo. `focus` exhibits the label name, a description, and the set of coordinates that the layer pulls into the repo. 

```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus available-layers
finagle (Finagle) -> ["//finagle/..."]
wilyns (wilyns) -> ["//wilyns/..."]
workflows (All workflows projects) -> ["//workflows/..."]
workflows/cdpain (Workflows cdpain project) -> ["//workflows/examples/cdpain/src:bin"]
```

### Writing a new layer

While I'm in my sparse repo, I figure out that I am missing something I'd like to hack on... `epoxy`. I define a new layer set in `focus/projects/my_team.layers.json` and check it in (after a quick review, I can land it, but I can use it meanwhile):

#### Contents of `focus/projects/my_team.layers.json`
```
{
    "layers": [
        {
            "name": "my_team/epoxy",
            "description": "All of epoxy",
            "coordinates": [
                "//epoxy/..."
            ]
        }
    ]
}
```

**Note**: Because of limitiations in Git (and the interest of sanity) `focus` can't operate in working trees with pending changes for some commands. Both dense and sparse repos need to clean.

After this change, `focus available-layers` reflects the new available layer.

```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus available-layers
finagle (Finagle) -> ["//finagle/..."]
wilyns (wilyns) -> ["//wilyns/..."]
my_team/epoxy (All of epoxy) -> ["//epoxy/..."]
workflows (All workflows projects) -> ["//workflows/..."]
workflows/cdpain (Workflows cdpain project) -> ["//workflows/examples/cdpain/src:bin"]
```

Let's push the new layer onto the stack.

```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus push-layer my_team/epoxy
0: finagle (Finagle) -> ["//finagle/..."]
1: my_team/epoxy (All of epoxy) -> ["//epoxy/..."]
```

Next, I need to update my working tree by running a `focus sync`.

```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus sync
[2021-09-16T18:28:15Z INFO  focus::subcommands::sync] Checking that dense repo is in a clean state
[2021-09-16T18:28:16Z INFO  focus::subcommands::sync] Checking that sparse repo is in a clean state
[2021-09-16T18:28:17Z INFO  focus::subcommands::sync] Enumerating coordinates
[2021-09-16T18:28:17Z INFO  focus::subcommands::sync] Determining the current commit in the dense repo
[2021-09-16T18:28:17Z INFO  focus::subcommands::sync] Determining the current commit in the sparse repo
[2021-09-16T18:28:17Z INFO  focus::subcommands::sync] Backing up the current sparse checkout file
[2021-09-16T18:28:17Z INFO  focus::subcommands::sync] Switching in the dense repo
[2021-09-16T18:28:18Z INFO  focus::subcommands::sync] Computing the new sparse profile
[2021-09-16T18:28:33Z INFO  focus::sparse_repos] Dependency query ["//tools/implicit_deps:thrift-implicit-deps-impl", "//scrooge-internal/...", "//loglens/loglens-logging/...", "//finagle/...", "//epoxy/..."] yielded 693 directories
[2021-09-16T18:28:33Z INFO  focus::subcommands::sync] Resetting in the dense repo
[2021-09-16T18:28:36Z INFO  focus::subcommands::sync] Applying the sparse profile
[2021-09-16T18:28:39Z INFO  focus::subcommands::sync] Updating the sync point
```

The `epoxy` targets are now in my tree to work on:
```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ bazel query //epoxy/...
INFO: Invocation ID: 50bdb321-dc7a-4d00-bf89-7fd3a8f3e5d7
//epoxy/src/test/scala/com/twitter/finagle/loadbalancer/aperture/epoxy:epoxy
//epoxy/src/test/scala/com/twitter/epoxy/xds:xds
//epoxy/src/test/scala/com/twitter/epoxy/udp:udp
...
```

After some hacking, `epoxy` isn't needed anymore, so I pop it and `sync` again:
```
wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus pop-layer
0: finagle (Finagle) -> ["//finagle/..."]

wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ focus sync
[2021-09-16T18:33:05Z INFO  focus::subcommands::sync] Checking that dense repo is in a clean state
[2021-09-16T18:33:06Z INFO  focus::subcommands::sync] Checking that sparse repo is in a clean state
[2021-09-16T18:33:07Z INFO  focus::subcommands::sync] Enumerating coordinates
[2021-09-16T18:33:07Z INFO  focus::subcommands::sync] Determining the current commit in the dense repo
[2021-09-16T18:33:07Z INFO  focus::subcommands::sync] Determining the current commit in the sparse repo
[2021-09-16T18:33:07Z INFO  focus::subcommands::sync] Backing up the current sparse checkout file
[2021-09-16T18:33:07Z INFO  focus::subcommands::sync] Switching in the dense repo
[2021-09-16T18:33:08Z INFO  focus::subcommands::sync] Computing the new sparse profile
[2021-09-16T18:33:09Z INFO  focus::sparse_repos] Dependency query ["//tools/implicit_deps:thrift-implicit-deps-impl", "//scrooge-internal/...", "//loglens/loglens-logging/...", "//finagle/..."] yielded 669 directories
[2021-09-16T18:33:09Z INFO  focus::subcommands::sync] Resetting in the dense repo
[2021-09-16T18:33:11Z INFO  focus::subcommands::sync] Applying the sparse profile
[2021-09-16T18:33:15Z INFO  focus::subcommands::sync] Updating the sync point

wilhelm at tw-mbp-wilhelm in ~/workspace/finagle
$ ls epoxy
ls: epoxy: No such file or directory
# epoxy is gone!
```


### What happens during sync?

Syncing lets the repository's shape change to reflect the files necessary to support the build coordinates implied by the currently selected set of layers.

It accomplishes this task by collecting all of the coordinates in the selected layers, getting the dense repository into the same state as the sparse repository, running the build graph query to determine the needed files and directories in the dense repository, and finally applying the changes in sparse repo.

*Note well*: the build graph computation cannot be performed in the sparse repo because only a partial build graph is present in the sparse repo. If a dependency is added that refers to a part of the graph that is not present in the sparse repo's build graph, it cannot be traversed, and the build system would fail. In the future, it would be nice to make this a remote operation (ideally with cached results to make it very fast), or to make the full build graph available through other means. 

