# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Prebuilt container for the `verify` job in .github/workflows/verify.yml.
#
# Contains everything that used to be installed via apt on every CI run:
#   - container base tools (ca-certificates, git, sudo, locales),
#   - the en_US.UTF-8 locale,
#   - the non-root CI user `kesticci`,
#   - all packages from `KESTIC_BOOTSTRAP_COMPONENTS=apt ./scripts/bootstrap.sh`.
#
# Rust toolchain, restic and test servers are NOT part of the image; they stay
# in the verify job (bootstrap.sh + actions/cache).
#
# The build context is `scripts/`. The image tag is a content hash of this file
# and scripts/bootstrap.sh (see .github/workflows/ci-image.yml), so verify.yml
# always runs in the image that matches the checked-out bootstrap.sh.
#
# The image must keep running as root: GitHub Actions executes container steps
# as the image user, and verify.yml needs root for chown and `su - kesticci`.

FROM ubuntu:26.04

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git sudo locales \
    && locale-gen en_US.UTF-8 \
    && useradd -m -s /bin/bash kesticci \
    && rm -rf /var/lib/apt/lists/*

COPY bootstrap.sh /opt/kestic-ci/scripts/bootstrap.sh

RUN KESTIC_BOOTSTRAP_COMPONENTS=apt bash /opt/kestic-ci/scripts/bootstrap.sh \
    && rm -rf /var/lib/apt/lists/*
