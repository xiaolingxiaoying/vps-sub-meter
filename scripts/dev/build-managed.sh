#!/usr/bin/env bash
set -euo pipefail
SRC=${SRC:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)}
DST=${DST:-$HOME/ws/vps-sub-meter-managed}
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/ws/target-vps-managed}
mkdir -p "$DST" "$SRC/.scratch/acceptance-bin"
tar -C "$SRC" --exclude=./target --exclude='./target-*' --exclude=./.git \
  --exclude=./.scratch --exclude=./dist --exclude=./node_modules --exclude=./.kilo -cf - . | tar -C "$DST" -xf -
cd "$DST"
cargo build --release --locked --no-default-features
cp "$CARGO_TARGET_DIR/release/sbctl" "$SRC/.scratch/acceptance-bin/sbctl-managed-linux-amd64"
gzip -c "$CARGO_TARGET_DIR/release/sbctl" > "$SRC/.scratch/acceptance-bin/sbctl-managed-linux-amd64.gz"
sha256sum "$SRC/.scratch/acceptance-bin/sbctl-managed-linux-amd64"
