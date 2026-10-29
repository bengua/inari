#!/usr/bin/env bash
# What must be true of this repository, by hand and in CI alike: the license is
# whole, and the Rust core is formatted, lint-clean, tested and builds. Rust only:
# nothing else is installed or run.
set -u
FAIL() { echo "$1"; echo "FAILED"; exit 1; }
DIR=$(cd "$(dirname "$0")" && pwd)
cd "$DIR"

for f in README.md LICENSE CHANGELOG.md CONTRIBUTING.md .gitignore; do
  [ -f "$f" ] || FAIL "missing: $f"
done
grep -qF 'Permission is hereby granted, free of charge' LICENSE || FAIL "LICENSE does not carry the MIT terms"
echo "OK  shape, MIT license"

# Rust: the toolchain is pinned by rust-toolchain.toml; a CI without one installs it.
if ! command -v cargo >/dev/null; then
  [ -n "${CI:-}${GITHUB_ACTIONS:-}" ] || FAIL "cargo absent: install rustup (https://rustup.rs)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none >/dev/null || FAIL "rustup install failed"
  . "$HOME/.cargo/env"
fi
cargo fmt --all -- --check || FAIL "rustfmt"
cargo clippy --all-targets --quiet -- -D warnings || FAIL "clippy"
cargo test --quiet || FAIL "cargo test"
cargo build --quiet || FAIL "cargo build"
echo "OK  rust: rustfmt, clippy, tests, build"

# English throughout, when english-check is installed (required where the CI says so:
# REQUIRE_ENGLISH_CHECK=1, or a CI image that carries it). Legitimately non-English
# files are listed in .english-exceptions with their reason.
if command -v english-check >/dev/null; then
  english-check --max 0 || FAIL "english-check: non-English text added (english-check --list)"
  echo "OK  english"
elif [ -n "${REQUIRE_ENGLISH_CHECK:-}" ] || [ -n "${FORGEJO_REPOSITORY:-}" ]; then
  FAIL "english-check absent"
else
  echo "SKIP english (english-check not installed)"
fi
echo "PASSED"
