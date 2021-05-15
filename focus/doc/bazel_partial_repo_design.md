# Bazel Partial Repo Design

Bazel is designed to operate with a complete view of the build
graph. In order to operate, Bazel needs access to all build
definitions (`BUILD` files) and the inputs to any rules -- most
frequently, source files.

As Bazel detects differences to the filesystem, it invalidates nodes
in its build graph in the background, notating as dirty those nodes
that have changed. Bazel stats files files that have been marked as
dirty and also performs content hashing (using SHA-256). Files with
identical metadata and contents are ignored as spurious. Internally,
these are represented using the class `ModifiedFileSet`.

Because Bazel needs a view of the entire filesystem and to snapshot
content, in order to support building in partial repositories, we need
to cheat a bit. To reiterate, ihe invariants that we are working with
are:

- We must be able to present the entire set of directory entries
  representing the tree:
  - File/directory name
  - The typical contents of the UNIX `struct stat`, namely:
    - Size in bytes and blocks on disk
    - Owner and group
  - Extended attributes
    - Permissions mask
  - Content hash (SHA-256)
- For `BUILD` files and other files they depend on, typically `.bzl`
  files, the entire contents of those files.

For project focused development, instead of having an entire set of
files materialized, we aim to only have those that are relevant to our
project. This puts us in a bind when a project references dependencies
outside its directories. Because it puts less load on tools, we would
like to treat all files outside of our directory as binary
dependencies. Provided all artifacts required for our project are
available in the build cache, we might be able to achieve the desired
effect by providing the metadata outlined above to Bazel by means of a
layered Bazel VFS implementation. Bazel's filesystem support (the code
in `bazel/src/main/java/com/google/devtools/build/lib/vfs`) is a
well-structured set of interfaces and implementations. We should build
an implementation of the `FileSystem` interface that allows a layered
approach. This would be strucutred as a further interface
`LayeredFileSystem` that is passed the underlying filesystem
implementation. A configuration parameter can be wired up that allows
for the instantiation of a class by name (using reflection) that
implements `LayeredFileSystem` for flexibility. I believe that these
patches could be upstreamed without too much work.

Rather than having all of these files on disk, we desire to transport
only what is necessary to inform Bazel of the state of the build
outside of our project directory. Therefore, we should create a
snapshot based on the contents of the SCM at the current merge base
containing this data. Entire copies of the files that Bazel needs to
operate (build definitions) will be present, whereas for most files,
it will only be metadata. If we use differential compression, the
amount of data necessary to represent most transitions in our repo
should be small. As part of the process of checking out files, the VCS
can provide this snapshot at a well-known location. Updates to these
snapshots could be applied in the background so that the user does not
need to wait for that operation as files in their working tree are
updated.

In the unlikely event that Bazel needs to read contents of files not
involved in the current project directory, we can fall back to reading
that content over the network. When these cases are detected, we
should consider whether we need to also include those resources in
snapshots. Our layered filesystem implementation should produce a log
of these instances for later analysis.
