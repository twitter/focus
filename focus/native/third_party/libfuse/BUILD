load("@rules_foreign_cc//tools/build_defs:make.bzl", "make")

make(
    name = "libfuse",
    binaries = ["fusermount3"],
    lib_source = "@libfuse//:all",
    make_commands = [
        "meson setup --prefix=$$INSTALLDIR$$ $$EXT_BUILD_ROOT$$ --backend=ninja",

        # For whatever reason Meson exits nonzero in the case of warnings or
        # something. Ignore it and let Bazel fail to find files.
        "meson install -C $$EXT_BUILD_ROOT$$ || true",

        # Meson, as with CMake, likes to put libraries in lib64, but Bazel
        # doesn't support that lifestyle choice.
        "cp -rv $$INSTALLDIR$$/lib64/* $$INSTALLDIR$$/lib",
    ],

    # TODO: Add Meson and Ninja toolchains
    make_env_vars = {},
    shared_libraries = ["libfuse3.so.3.10.1"],
    visibility = ["//visibility:public"],
)
