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
signer=${VSM_SIGNER:-${SBCTL_SIGNER:-$(command -v vps-sub-meter || true)}}
[[ -n "$signer" ]] || { echo "missing vps-sub-meter signer binary" >&2; exit 2; }

bin_sha=$(sha256sum "$bin_file" | awk '{print $1}')
bundle_sha=$(sha256sum "$bundle_file" | awk '{print $1}')
base="https://github.com/${repo}/releases/download/${tag}"
ver=${tag#v}
ver=${ver#sbctl-v}

unsigned="$output.unsigned.tmp"
trap 'rm -f "$unsigned"' EXIT

jq -n \
  --argjson schema 1 \
  --arg ver "$ver" \
  --arg bin_url "$base/vps-sub-meter-linux-$arch" \
  --arg bin_sha "$bin_sha" \
  --arg bundle_url "$base/vps-sub-meter-bundle.tar.gz" \
  --arg bundle_sha "$bundle_sha" \
  '{schema:$schema,vps_sub_meter:{version:$ver,url:$bin_url,sha256:$bin_sha},bundle:{version:$ver,url:$bundle_url,sha256:$bundle_sha}}' \
  > "$unsigned"

"$signer" release sign --manifest "$unsigned" --private-key "$signing_key" --output "$output"
"$signer" release verify --manifest "$output"
