#!/usr/bin/env bash
# Narrow WSL runner for a single sbctl CLI integration test filter.
# Mirrors scripts/dev/wsl-gate.sh's sync step but skips fmt/clippy so the
# Linux-only test can iterate quickly. Usage:
#   wsl -d Ubuntu-22.04 -- bash /mnt/c/.../scripts/dev/wsl-one-test.sh <filter>
set -euo pipefail

SRC=${SRC:-/mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter}
DST=${DST:-/home/ly/ws/vps-sub-meter}
FILTER=${1:?usage: wsl-one-test.sh <test-name-filter>}

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "chosen branch: SRC=$SRC DST=$DST FILTER=$FILTER"

mkdir -p "$DST"
synced_at=$(date +%s)
tar -C "$SRC" \
  --exclude=./target --exclude='./target-*' --exclude=./.git \
  --exclude='./.reference-* --exclude=./.scratch' \
  --exclude=./dist --exclude=./node_modules \
  --exclude=./.zcode --exclude=./.kilo --exclude=./.qoder \
  -cf - . | tar -C "$DST" -xf -
echo "synced to $DST in $(( $(date +%s) - synced_at ))s"

cd "$DST"
CARGO_TARGET_DIR="$HOME/ws/target" cargo test -p sbctl --test cli \
  --features test-signing "$FILTER" -- --nocapture
