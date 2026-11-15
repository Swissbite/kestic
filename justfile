# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

default:
    @just --list

# Run all verification checks (AC-M0-01-1)
verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --bin kestic
    cargo test --workspace
    cargo deny check
    reuse lint

fmt:
    cargo fmt --all

test:
    cargo build --bin kestic
    cargo test --workspace
