#!/usr/bin/env bash
# Release gate: a publishable sbctl build must refuse the publicly known
# development signing key.
#
# The positive direction - that the binary accepts the production key - is
# proven by the package job, which is the only place the production private seed
# exists: `scripts/generate-manifest.sh` signs a manifest with it and then
# verifies that manifest with the release binary it just built.
#
# This negative direction is what a `--features test-signing` build or a build
# with no configured anchor fails. Byte-grepping the binary for the key is not
# enough: the production anchor is embedded as a hex string while the
# development key is a byte array, so the check is behavioural.
#
# usage: release-trust-anchor-check.sh BINARY [DEV_SEED_FILE]
set -euo pipefail

binary=${1:?usage: release-trust-anchor-check.sh BINARY [DEV_SEED_FILE]}
repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
seed=${2:-$repository_root/scripts/dev-signing-key.hex}

if [[ ! -x "$binary" ]]; then
  echo "release gate failed: $binary is not an executable file" >&2
  exit 2
fi
if [[ ! -f "$seed" ]]; then
  echo "release gate failed: development seed $seed is missing" >&2
  exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# A fully valid manifest - the gate must fail on the key, not on a field.
cat >"$work/manifest.json" <<'EOF'
{"schema":1,"sbctl":{"version":"0.0.1","url":"https://example.test/sbctl","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"sing_box":{"version":"1.14.2","url":"https://example.test/sing-box","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"sing_box_compatibility":[{"min":"1.10.0","max":"1.14.99"}]}
EOF

"$binary" release sign --manifest "$work/manifest.json" --private-key "$seed" --output "$work/signed.json" >/dev/null

if "$binary" release verify --manifest "$work/signed.json" >"$work/out" 2>&1; then
  echo "release gate failed: $binary trusts the publicly known development key and must not be published" >&2
  exit 1
fi

# A build without a configured anchor also refuses every manifest, but it cannot
# verify anything a release needs to verify. Distinguish the two: the anchor
# must be present, and the refusal above must come from the signature.
if grep -q 'no production release public key was configured at build time' "$work/out"; then
  echo "release gate failed: $binary was built without SBCTL_RELEASE_PUBLIC_KEY_HEX, so it cannot verify any release" >&2
  exit 1
fi
if ! grep -q 'signature is invalid' "$work/out"; then
  echo "release gate failed: $binary refused the development key for an unexpected reason:" >&2
  cat "$work/out" >&2
  exit 1
fi

# A malformed manifest must still be rejected, so the gate cannot pass because
# verification is broken for every input.
printf '%s' '{"schema":2}' >"$work/invalid.json"
if "$binary" release verify --manifest "$work/invalid.json" >/dev/null 2>&1; then
  echo "release gate failed: $binary accepted a schema-2 manifest" >&2
  exit 1
fi

echo "release gate passed: $binary has a production anchor, refuses the development key, and rejects an invalid manifest"
