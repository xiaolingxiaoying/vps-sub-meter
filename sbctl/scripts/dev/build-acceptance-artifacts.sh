#!/usr/bin/env bash
# Build the Linux binaries the systemd acceptance suite consumes:
#   SBCTL_ARTIFACT        release binary            -> .scratch/acceptance-bin/sbctl-linux-amd64
#   SBCTL_TEST_ARTIFACT   test-signing fixture     -> .scratch/acceptance-bin/sbctl-test-signing
#   SBCTUI_ARTIFACT       release client binary    -> .scratch/acceptance-bin/sbtui-linux-amd64
# The fixture build is never published (see docs/release-signing.md).
#
# `tests/acceptance/run.sh` treats the two client artifacts as optional: without
# them it prints `branch: server-only workspace - skipping the client legs` and
# runs the three server assertions (`verify-bootstrap.sh`, `verify.sh`,
# `verify-real.sh`). Export SBCTUI_ARTIFACT/SBCLI_ARTIFACT from a singbox-sub-me
# build to run the full suite, including `verify-client.sh` (orphan reclamation
# and the TUN wiring) and `verify-sbcli.sh` (shared daemon protocol). The script
# says out loud which branch it took, so a missing client binary never reads as
# a broken build.
#
# Run inside WSL. The Windows tree is synced into the Linux filesystem first so
# the compiler never touches /mnt/c, and the outputs are copied back only at the
# end because Docker Desktop cannot bind a WSL path from the Windows CLI.
set -euo pipefail

SRC=${SRC:-/mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter}
DST=${DST:-$HOME/ws/vps-sub-meter}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/ws/target-vps}

# Non-login shells do not read ~/.cargo/env, and this script is often invoked
# straight through `wsl -- bash …`.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

# SRC_REV=HEAD builds exactly what is committed instead of the working tree.
# Worth having: with another process editing a crate, a dirty tree can fail the
# build for reasons unrelated to the artifacts under test, and an acceptance run
# should describe a commit rather than a moment.
if [ -n "${SRC_REV:-}" ]; then
  DST="$HOME/ws/acceptance-src"
  export CARGO_TARGET_DIR="$HOME/ws/acceptance-target"
  rm -rf "$DST"
  mkdir -p "$DST"
  git -C "$SRC" archive "$SRC_REV" | tar -x -C "$DST"
  echo "exported revision $SRC_REV into $DST"
else
  mkdir -p "$DST"
  tar -C "$SRC" \
    --exclude=./target --exclude='./target-*' --exclude=./.git \
    --exclude='./.reference-*' --exclude=./.scratch --exclude=./dist \
    --exclude=./.tmp-sing-box-yg-research --exclude=./node_modules \
    -cf - . | tar -C "$DST" -xf -
fi

cd "$DST"
echo "=== release build (no test-signing) ==="
cargo build --release --locked -p sbctl --no-default-features
echo "=== fixture build (test-signing) ==="
cargo build --release --locked -p sbctl --features test-signing --target-dir target-fixtures
echo "=== client build (verify-client.sh) ==="
if cargo metadata --no-deps --format-version 1 | grep -q '"sbtui"'; then
  echo "branch: sbtui/sbcli are in this workspace - building them"
  cargo build --release --locked -p sbtui -p sbcli
else
  echo "branch: server-only workspace - skipping the client leg"
  echo "  run.sh still runs the three server assertions; export SBCTUI_ARTIFACT" >&2
  echo "  and SBCLI_ARTIFACT from a singbox-sub-me build to add the client ones." >&2
fi

out="$SRC/.scratch/acceptance-bin"
mkdir -p "$out"
cp "$CARGO_TARGET_DIR/release/sbctl" "$out/sbctl-linux-amd64"
cp target-fixtures/release/sbctl "$out/sbctl-test-signing"
if [ -f "$CARGO_TARGET_DIR/release/sbtui" ]; then
  cp "$CARGO_TARGET_DIR/release/sbtui" "$out/sbtui-linux-amd64"
  cp "$CARGO_TARGET_DIR/release/sbcli" "$out/sbcli-linux-amd64"
fi
echo "=== artifacts ==="
ls -l "$out"
for binary in sbctl-linux-amd64 sbctl-test-signing sbtui-linux-amd64 sbcli-linux-amd64; do
  [ -f "$out/$binary" ] || continue
  file "$out/$binary" | sed 's/^/  /'
done
