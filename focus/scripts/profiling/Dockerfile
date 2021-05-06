# TODO: Add ONBUILD directives where appropriate

# Based on the file from Pants (build-support/docker/centos6)

# Copyright 2017 Pants project contributors (see CONTRIBUTORS.md).
# Licensed under the Apache License, Version 2.0 (see LICENSE).

ARG USER

FROM centos:7

ENV toolset_base devtoolset-9

COPY vbatts-bazel-epel-7.repo /etc/yum.repos.d

RUN adduser -m profiler_user && \
    usermod -aG wheel profiler_user
RUN yum clean all && \
    rm -r /var/cache/yum 
RUN yum upgrade -y  && \
    yum install -y centos-release-scl
RUN yum install -y \
        $toolset_base \
        $toolset_base-* \
        autoconf \
        automake \
        bazel3 \
        bzip2-devel \
        cmake \
        git \
        java-1.8.0-openjdk-devel \
        joe \
        libffi-devel \
        openssl-devel \
        readline-devel \
        sqlite-devel \
        yum-utils \
        zlib-devel

##
## Ninja for CMake builds
##
# Install Ninja
RUN cd /usr/local/bin && \
    curl -L -O https://github.com/ninja-build/ninja/releases/download/v1.10.2/ninja-linux.zip && \
    unzip ninja-linux.zip && \
    rm ninja-linux.zip

##
## Python 2 and 3 toolchains for Pants
##
ARG PYTHON_27_VERSION=2.7.13
ARG PYTHON_36_VERSION=3.6.8
# NB: PYENV_ROOT must be set for `pyenv install` to be available. This failure mode is not mentioned
# in https://github.com/pyenv/pyenv#basic-github-checkout.
ENV PYENV_ROOT /pyenv-docker-build
ENV PYENV_BIN "${PYENV_ROOT}/bin/pyenv"
RUN git clone https://github.com/pyenv/pyenv ${PYENV_ROOT}

# Install Python 2.7 and 3.6.
# NB: We intentionally do not use `--enable-shared`, as it results in our shipped wheels for the PEX release using
# `libpython.X.Y.so.1`, which means that the PEX will not work for any consumer interpreters that were statically
# built, i.e. compiled without `--enable-shared`. See https://github.com/pantsbuild/pants/pull/7794.
RUN /usr/bin/scl enable $toolset_base -- ${PYENV_BIN} install ${PYTHON_27_VERSION}
RUN /usr/bin/scl enable $toolset_base -- ${PYENV_BIN} install ${PYTHON_36_VERSION}
RUN ${PYENV_BIN} global ${PYTHON_27_VERSION} ${PYTHON_36_VERSION}
ENV PATH "${PYENV_ROOT}/shims:${PATH}"

##
## Meson toolchain (libfuse build uses this)
##
RUN pip3 install meson

##
## Rust nightly toolchain
##
# From https://github.com/linkerd/rustup-nightly-docker/blob/master/Dockerfile
ENV RUSTUP_HOME=/rust
ENV CARGO_HOME=/cargo 
ENV PATH=/cargo/bin:/rust/bin:$PATH

RUN echo "(curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain nightly --no-modify-path) && rustup default nightly" > /install-rust.sh && chmod 755 /install-rust.sh
RUN /usr/bin/scl enable $toolset_base -- /install-rust.sh

##
## Flamegraph for visualization
##
RUN git clone https://github.com/brendangregg/FlameGraph /opt/flamegraph

##
## Misc utilities
##
RUN yum install -y sudo

ENTRYPOINT ["/usr/bin/scl", "enable", "devtoolset-9",  "--"]

# TODO: figure out why this needs to be installed first for /usr/bin/scl to work!
