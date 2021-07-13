Git sparse checkouts provide for maintaining a working tree that contains only paths matching a set of predicate expressions.  For Project Focused Development (PFD) we aim to limit the files present in the working tree to those directly related to the project at hand. This means that there is an incomplete set of files on disk.

Imagine that, in the Source repository, we only want to work on the Workflows project.  A sparse profile would be used that specifies only the `workflows` directory.  Given a repository working tree that only contains `workflows`, Bazel can obviously not build the project.  Since the `WORKSPACE` file and `tools` directory are absent, Bazel will not even recognize the working tree as a valid Bazel workspace.  In order to make this workspace buildable, those file need to be addressable by Bazel.

The virtual filesystem (VFS) is meant to allow for partial contents to exist on disk, as is the case in a sparse checkout.

For Bazel to work, we need a full complement of WORKSPACE, BUILD, and other referenced files.  We will refer to these as the build-related files.  Additionally, we need checksums and metadata of all other files in the repository to act as placeholders.  Bazel

The filesystem image is encoded using the format in focus/formats/proto/treesnap.proto. These are streamed out to disk using the `focus/internals/proto_file_stream` library, which uses the following simple length-delimited framing:


```
+-[entry]----------------+
| message_length: uint32 |
| message_bytes: bytes   |
+------------------------+

+-[file]-----------------+
| [entry1] [entryN] ...  |
+------------------------+
```
