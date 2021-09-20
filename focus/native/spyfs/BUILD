load("@rules_cc//cc:defs.bzl", "cc_binary", "cc_library", "cc_test")

cc_library(
    name = "annotations",
    srcs = [],
    hdrs = ["annotations.h"],
    deps = [],
)

cc_library(
    name = "locking",
    srcs = ["locking.cpp"],
    hdrs = ["locking.h"],
    deps = [
        ":annotations",
        "@com_google_absl//absl/synchronization",
    ],
)

cc_library(
    name = "tablet",
    srcs = [ "tablet.cpp" ],
    hdrs = [ "tablet.h" ],
    deps = [
        ":annotations",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
        "@com_google_absl//absl/base",
        "@com_google_absl//absl/container:flat_hash_set",
        "@com_google_absl//absl/container:flat_hash_map",
        "@com_google_absl//absl/synchronization",
        "@com_google_absl//absl/strings",
    ],
)

cc_test(
    name = "tablet_test",
    size = "small",
    srcs = [
        "tablet_test.cpp",
    ],
    deps = [
        ":tablet",
        ":test_util",
        "@com_google_googletest//:gtest_main",
    ],
)

cc_binary(
    name = "spyfs",
    srcs = ["fs.cpp"],
    copts = [
        "-D_FILE_OFFSET_BITS=64",
        "-DHAVE_UTIMENSAT",
    ],
    deps = [
        ":annotations",
        ":moniker",
        ":tablet",
        ":tracker",
        "//third_party/libfuse",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
        "@com_google_absl//absl/base",
        "@com_google_absl//absl/container:flat_hash_set",
        "@com_google_absl//absl/container:flat_hash_map",
        "@com_google_absl//absl/synchronization",
        "@com_google_absl//absl/strings",
    ],
)

cc_library(
    name = "moniker",
    srcs = [
        "moniker.cpp",
    ],
    hdrs = ["moniker.h"],
    deps = [
        ":annotations",
        ":locking",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
        "@com_google_absl//absl/base",
        "@com_google_absl//absl/container:flat_hash_map",
        "@com_google_absl//absl/container:node_hash_set",
        "@com_google_absl//absl/container:node_hash_map",
        "@com_google_absl//absl/hash",
        "@com_google_absl//absl/strings:str_format",
        "@com_google_absl//absl/synchronization",
    ],
)

cc_test(
    name = "moniker_test",
    size = "small",
    srcs = [
        "moniker_test.cpp",
    ],
    deps = [
        ":moniker",
        ":test_util",
        "@com_google_googletest//:gtest_main",
    ],
)

cc_library(
    name = "tracker",
    srcs = [
        "tracker.cpp",
    ],
    hdrs = ["tracker.h"],
    deps = [
        ":annotations",
        ":locking",
        ":moniker",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
        "@com_google_absl//absl/base",
        "@com_google_absl//absl/container:flat_hash_map",
        "@com_google_absl//absl/container:node_hash_set",
        "@com_google_absl//absl/hash",
        "@com_google_absl//absl/strings:str_format",
        "@com_google_absl//absl/synchronization",
    ],
)

cc_test(
    name = "tracker_test",
    size = "small",
    srcs = [
        "tracker_test.cpp",
    ],
    deps = [
        ":tracker",
        ":test_util",
        "@com_google_googletest//:gtest_main",
    ],
)

cc_binary(
    name = "tracker_demo",
    srcs = ["tracker_demo.cpp"],
    deps = [
        ":tracker",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
    ],
)

cc_library(
    name = "test_util",
    srcs = ["test_util.cpp"],
    hdrs = ["test_util.h"],
    deps = [
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
        "@com_google_googletest//:gtest",
    ],
)


cc_test(
    name = "test_util_test",
    size = "small",
    srcs = [
        "test_util_test.cpp",
    ],
    deps = [
        ":test_util",
        "@com_google_googletest//:gtest_main",
        "@com_github_gflags_gflags//:gflags",
        "@com_github_google_glog//:glog",
    ],
)
