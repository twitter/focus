load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

def bazel_deps_repository():
    commit = "38d2ca6594f8fd886a11ffa4e03d874d83083e9e"
    http_archive(
        name = "com_github_mjbots_bazel_deps",
        url = "https://github.com/mjbots/bazel_deps/archive/{}.zip".format(commit),
        # Try the following empty sha256 hash first, then replace with whatever
        # bazel says it is looking for once it complains.
        sha256 = "1704dd8c4e0bfb869451dfe1412dc25804db0e49335676042e760a855c3e25a8",
        strip_prefix = "bazel_deps-{}".format(commit),
    )
    