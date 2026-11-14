#!/usr/bin/env bash
# Windows/WSL verification in an isolated native-filesystem checkout.
set -euo pipefail
SRC=${SRC:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)}
DST=${DST:-$HOME/ws/vps-sub-meter-managed}
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/ws/target-vps-managed}
mkdir -p "$DST"
tar -C "$SRC" --exclude=./target --exclude='./target-*' --exclude=./.git \
  --exclude=./.scratch --exclude=./dist --exclude=./node_modules --exclude=./.kilo -cf - . | tar -C "$DST" -xf -
cd "$DST"
mkdir -p .test-cores
archive="$SRC/.scratch/test-runtimes/Xray-linux-64.zip"
printf '%s  %s\n' e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214 "$archive" | sha256sum -c -
python3 - "$archive" .test-cores/xray <<'PY'
import sys, zipfile
from pathlib import Path
with zipfile.ZipFile(sys.argv[1]) as archive:
    Path(sys.argv[2]).write_bytes(archive.read('xray'))
PY
chmod +x .test-cores/xray
export SBCTL_XRAY_BIN="$DST/.test-cores/xray"
export SING_BOX_BIN_1_10="$HOME/bin/sing-box-1.10.7"
export SING_BOX_BIN_1_11="$HOME/bin/sing-box-1.11.15"
export SING_BOX_BIN_1_12="$HOME/bin/sing-box-1.12.25"
export SING_BOX_BIN_1_13="$HOME/bin/sing-box-1.13.21"
export SING_BOX_BIN_1_14="$HOME/bin/sing-box-1.14.1"
export MIHOMO_BIN="${MIHOMO_BIN:-$HOME/bin/mihomo}"
cargo test --locked --workspace --features sbctl/test-signing --quiet
cargo test --locked --test managed_deployment -- --ignored --nocapture
cargo test --locked --test xray_core -- --ignored --nocapture
cargo test --locked --test preset_connections tls_presets_carry_http -- --ignored --nocapture
SBCTL_CLIENT_BIN="$SING_BOX_BIN_1_14" cargo test --locked --test preset_connections sing_box_compatible -- --ignored --nocapture
python3 tests/acceptance/verify-menu.py "$CARGO_TARGET_DIR/debug/sbctl"
python3 tests/acceptance/verify-managed-menu.py "$CARGO_TARGET_DIR/debug/sbctl"
if [ "${BUILD_MANAGED_CANDIDATE:-0}" = 1 ]; then
  DST="$DST" CARGO_TARGET_DIR="$CARGO_TARGET_DIR" bash "$SRC/scripts/dev/build-managed.sh"
fi
