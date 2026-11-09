#!/usr/bin/env bash
# Run under WSL Ubuntu 22.04. Compile on its native filesystem, then return
# executables to the Windows workspace; never invoke Cargo on a VPS.
set -euo pipefail
test "$(uname -m)" = x86_64 || { echo 'This candidate bundle targets Linux amd64' >&2; exit 1; }
SRC=${SRC:-/mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter}
DST=${DST:-$HOME/ws/vps-sub-meter}
OUT="$SRC/.scratch/acceptance-bin"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/ws/target-vps}
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
mkdir -p "$DST" "$OUT"
tar -C "$SRC" --exclude=./target --exclude='./target-*' --exclude=./.git \
  --exclude=./.scratch --exclude=./dist --exclude=./node_modules \
  --exclude=./.kilo --exclude=./.zcode --exclude=./.qoder -cf - . | tar -C "$DST" -xf -
cd "$DST"
cargo build --release --locked -p sbctl --no-default-features
cp "$CARGO_TARGET_DIR/release/sbctl" "$OUT/sbctl-linux-amd64"
cargo test --release --locked --no-default-features --test xray_core \
  --test preset_connections --no-run --message-format=json > "$OUT/build-tests.jsonl"
python3 - "$OUT/build-tests.jsonl" "$OUT" <<'PY'
import json, shutil, sys
from pathlib import Path
out = Path(sys.argv[2])
for line in Path(sys.argv[1]).read_text().splitlines():
    item = json.loads(line)
    if item.get('reason') == 'compiler-artifact' and item.get('executable') and item['target']['name'] in ('xray_core', 'preset_connections'):
        shutil.copy2(item['executable'], out / (item['target']['name'] + '-linux-amd64'))
PY
sha256sum "$OUT/sbctl-linux-amd64" "$OUT/xray_core-linux-amd64" "$OUT/preset_connections-linux-amd64"
