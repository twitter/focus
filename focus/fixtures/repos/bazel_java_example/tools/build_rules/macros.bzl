# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0

load("@rules_java//java:defs.bzl", "java_binary")

def my_java_binary(**kwargs):
    java_binary(**kwargs)
