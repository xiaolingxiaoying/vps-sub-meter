#!/usr/bin/env bash
# Sync the working tree into the WSL filesystem, then run the same gates as the
# Linux CI job. Everything is compiled and executed inside WSL; the Windows host
# only serves the source mount.
#
# Usage (from Git Bash):
#   MSYS_NO_PATHCONV=1 wsl -d Ubuntu-22.04 -- bash <repo>/scripts/dev/wsl-gate.sh
#
# Narrow a run while iterating:
#   TEST_ARGS='--test cli update' SKIP_CLIPPY=1 bash scripts/dev/wsl-gate.sh
#
# This workspace is the server only: `sbctl` plus `crates/json-merge`. The GUI
# crates that the monorepo's gate had to exclude (Qt toolchain) are not here, so
# the workspace checks need no exceptions.
set -euo pipefail

SRC=${SRC:-/mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter}
DST=${DST:-$HOME/ws/vps-sub-meter}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/ws/target-vps}
TEST_ARGS=${TEST_ARGS:-}
SKIP_CLIPPY=${SKIP_CLIPPY:-0}
SKIP_TESTS=${SKIP_TESTS:-0}

# `wsl -- bash script.sh` is a non-login shell, so it never reads ~/.cargo/env
# the way `wsl -e bash -lc` does. Without this the gate dies at its first cargo
# line with "cargo: command not found", which reads like a broken toolchain.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

mkdir -p "$DST"
echo "chosen branch: SRC=$SRC DST=$DST CARGO_TARGET_DIR=$CARGO_TARGET_DIR TEST_ARGS='${TEST_ARGS}' SKIP_CLIPPY=$SKIP_CLIPPY SKIP_TESTS=$SKIP_TESTS"
synced_at=$(date +%s)
tar -C "$SRC" \
  --exclude=./target --exclude='./target-*' --exclude=./.git \
  --exclude=./.reference-* --exclude=./.tmp-sing-box-yg-research \
  --exclude=./.scratch --exclude=./dist --exclude=./node_modules \
  --exclude=./.zcode --exclude=./.kilo --exclude=./.qoder \
  -cf - . | tar -C "$DST" -xf -
echo "synced to $DST in $(( $(date +%s) - synced_at ))s (target: $CARGO_TARGET_DIR)"
cd "$DST"

echo "=== fmt ==="
cargo fmt --all -- --check

if [ "$SKIP_CLIPPY" != "1" ]; then
  echo "=== clippy ==="
  cargo clippy --workspace --all-targets --all-features -- -D warnings
fi

if [ "$SKIP_TESTS" != "1" ]; then
  echo "=== tests ==="
  # shellcheck disable=SC2086
  cargo test --workspace --features sbctl/test-signing $TEST_ARGS
fi

echo "wsl gate passed"
