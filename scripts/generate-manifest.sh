#!/usr/bin/env bash
set -euo pipefail

# usage: generate-manifest.sh OWNER/REPO TAG ARCH BIN_FILE BUNDLE_FILE OUTPUT
if [[ "$#" -ne 6 ]]; then
  echo "usage: generate-manifest.sh OWNER/REPO TAG ARCH BIN_FILE BUNDLE_FILE OUTPUT" >&2
  exit 2
fi

repo=$1
tag=$2
arch=$3
bin_file=$4
bundle_file=$5
output=$6

signing_key=${VSM_SIGNING_KEY:-${SBCTL_SIGNING_KEY:?set VSM_SIGNING_KEY or SBCTL_SIGNING_KEY}}
pub_hex=${VSM_RELEASE_PUBLIC_KEY_HEX:-${SBCTL_RELEASE_PUBLIC_KEY_HEX:-}}

bin_sha=$(sha256sum "$bin_file" | awk '{print $1}')
bundle_sha=$(sha256sum "$bundle_file" | awk '{print $1}')
base="https://github.com/${repo}/releases/download/${tag}"
ver=${tag#v}
ver=${ver#sbctl-v}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

jq -n \
  --argjson schema 1 \
  --arg ver "$ver" \
  --arg bin_url "$base/vps-sub-meter-linux-$arch" \
  --arg bin_sha "$bin_sha" \
  --arg bundle_url "$base/vps-sub-meter-bundle.tar.gz" \
  --arg bundle_sha "$bundle_sha" \
  '{schema:$schema,vps_sub_meter:{version:$ver,url:$bin_url,sha256:$bin_sha},bundle:{version:$ver,url:$bundle_url,sha256:$bundle_sha}}' \
  > "$work/unsigned.json"

jq -S -c 'del(.signature)' "$work/unsigned.json" | tr -d '\r\n' > "$work/canonical.json"

python3 - "$signing_key" "$work/private.pem" "$work/public.pem" "$pub_hex" <<'PY'
import base64
import sys
from pathlib import Path

seed_path, priv_out, pub_out, pub_hex = sys.argv[1:5]
seed = bytes.fromhex(Path(seed_path).read_text(encoding="utf-8").strip())
if len(seed) != 32:
    raise SystemExit("signing seed must be 32-byte hex")
pkcs8 = bytes.fromhex("302e020100300506032b657004220420") + seed
priv_pem = "-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(pkcs8).decode() + "\n-----END PRIVATE KEY-----\n"
Path(priv_out).write_text(priv_pem, encoding="utf-8")
if pub_hex.strip():
    pub = bytes.fromhex(pub_hex.strip())
    if len(pub) != 32:
        raise SystemExit("public key must be 32-byte hex")
    spki = bytes.fromhex("302a300506032b6570032100") + pub
    pub_pem = "-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(spki).decode() + "\n-----END PUBLIC KEY-----\n"
    Path(pub_out).write_text(pub_pem, encoding="utf-8")
PY

if [[ ! -s "$work/public.pem" ]]; then
  openssl pkey -in "$work/private.pem" -pubout -out "$work/public.pem" 2>/dev/null
fi

openssl pkeyutl -sign -inkey "$work/private.pem" -rawin -in "$work/canonical.json" -out "$work/sig.bin"
openssl pkeyutl -verify -pubin -inkey "$work/public.pem" -rawin -in "$work/canonical.json" -sigfile "$work/sig.bin" >/dev/null
sig_b64=$(base64 -w0 "$work/sig.bin" 2>/dev/null || base64 "$work/sig.bin" | tr -d '\r\n')

jq --arg sig "$sig_b64" '. + {signature:$sig}' "$work/unsigned.json" > "$output"
