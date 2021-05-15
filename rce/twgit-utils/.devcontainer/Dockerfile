# See here for image contents: https://github.com/microsoft/vscode-dev-containers/tree/v0.177.0/containers/debian/.devcontainer/base.Dockerfile

ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# [Choice] Debian version: buster, stretch
ARG VARIANT="buster"
FROM mcr.microsoft.com/vscode/devcontainers/base:0-${VARIANT} AS base

# ** [Optional] Uncomment this section to install additional packages. **
RUN apt-get update \
    && export DEBIAN_FRONTEND=noninteractive \
    && apt-get -y install --no-install-recommends \
        autoconf \
        bison \
        build-essential \
        curl \
        libbz2-dev \
        libdb-dev \
        libffi-dev \
        libgdbm-dev \
        libgdbm6 \
        liblzma-dev \
        libncurses5-dev \
        libreadline-dev \
        libsqlite3-dev \
        libssl-dev \
        libxml2-dev \
        libxmlsec1-dev \
        libyaml-dev \
        llvm \
        make \
        tk-dev \
        wget \
        xz-utils \
        zlib1g-dev \
        sudo \
        direnv \
    && apt-get clean -y \
    && apt-get autoclean -y \
    && rm -rf /var/lib/apt/lists/*

COPY install-go.sh /tmp/install-go.sh

FROM base as golangtools
RUN cd /tmp \
    && sudo bash /tmp/install-go.sh \
    && rm /tmp/install-go.sh

