load("//tools/workspace/bazel_deps:repository.bzl", "bazel_deps_repository")

def add_default_repositories(excludes = []):
    if "bazel_deps" not in excludes:
        bazel_deps_repository()
        