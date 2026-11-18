#!/usr/bin/env bash
# Download CI fixtures into the working directory. Run from the repository root.
set -euo pipefail
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

fetch() {
  curl --fail --silent --show-error --location --retry 4 --connect-timeout 20 \
    --max-time 240 --proto '=https' --proto-redir '=https' --output "$work/archive" "$1"
  printf '%s  %s\n' "$2" "$work/archive" | sha256sum --check --status || {
    echo "CI fixture checksum mismatch: $1" >&2
    exit 1
  }
}

mihomo() {
  fetch "https://github.com/MetaCubeX/mihomo/releases/download/v$1/$2.gz" "$3"
  gzip -dc "$work/archive" > "$work/bin"
  install -Dm755 "$work/bin" "$4"
}

case ${1:-} in
  sing-box)
    while read -r version digest; do
      name="sing-box-$version-linux-amd64"
      fetch "https://github.com/SagerNet/sing-box/releases/download/v$version/$name.tar.gz" "$digest"
      tar -xzOf "$work/archive" -- "$name/sing-box" > "$work/bin"
      install -Dm755 "$work/bin" "sing-box-$version/sing-box"
    done <<'PINS'
1.10.7 1951a0785c8b4e1e21e0640227a49528ca772aec3d680061652e3d6b687e00fe
1.11.15 950af37eb2d7e55dddae34a18411cd617303fd99d2dc75bc76b6dd9fcd97d9c5
1.12.25 a1ec76e2b6b139eb747a1b1ebee7d14b8d4be5a833596cad8070a31ef960301f
1.13.21 24f9ef8e7234e13e71e74c3598a4164c5fe07b7b67ccc6e96cf68b54789f72cd
1.14.1 12cb2816b52febb356f6a885b740cc8758c3f30b8ae0ca8edba80f0d2d35343f
1.14.4 699a1b510aa4396b2176d32d1181920399ebf1e267835da6f3b598d0f7c24968
PINS
    ;;
  managed)
    fetch https://github.com/XTLS/Xray-core/releases/download/v26.10.31/Xray-linux-64.zip e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214
    unzip -p "$work/archive" xray > "$work/bin"
    install -Dm755 "$work/bin" managed-cores/xray
    mihomo 1.19.33 mihomo-linux-amd64-compatible-v1.19.33 386508bfb1c72c43b6a0eda73708ecdce82546581b0ebcb16366c53757c622b6 managed-cores/mihomo
    ;;
  mihomo)
    mihomo 1.19.31 mihomo-linux-amd64-compatible-v1.19.31 04cf9f09671704f839ddbee2e93069dc831a4123a75281e725d1d96ab9ac1afc mihomo/mihomo
    mihomo 1.18.10 mihomo-linux-amd64-v1.18.10 362467189fc580ab680bad25b9040eb6f5a54ff534e5d37065165912a914f3f3 mihomo/mihomo-legacy
    ;;
  *) echo 'Usage: bash scripts/dev/ci-cores.sh {sing-box|managed|mihomo}' >&2; exit 2 ;;
esac
