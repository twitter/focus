# Implementing Project Focused Development with Cloud Build Graph

## Invariants
- We desire to work with projections of the repository that are as small as can be to accomplish the task at hand: often times, a single project directory in _Source_.
- It should be easy to expand and shrink the contents of a repository.
- Workstations should not be saddled with compiling or extracting semantic information from sources unrelated to those projects being focused on.

Because our chosen build system, Bazel, requires the ability to address all `BUILD` definitions and a full complement of filesystem metadata for those files and directories involved in the build, it is difficult to know which files are required for a sparse clone of the repository when starting from scratch.

A naive approach to solving this might be to have full copy of the repository for reference, and to spawn smaller workspaces after querying the build graph.