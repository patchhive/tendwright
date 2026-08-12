#!/usr/bin/env bash
set -euo pipefail

# All monorepo Rust packages belong to the root workspace. Standalone export
# lockfiles are release artifacts and are deliberately not CI inputs here.

echo "::group::cargo fmt --all"
cargo fmt --all -- --check
echo "::endgroup::"

echo "::group::cargo clippy --workspace"
cargo clippy --locked --workspace --all-targets -- -D warnings
echo "::endgroup::"

echo "::group::cargo test --workspace"
cargo test --locked --workspace --all-targets
echo "::endgroup::"
