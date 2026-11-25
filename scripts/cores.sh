#!/usr/bin/env bash
set -euo pipefail

# Official dual-core installer/updater with strict SHA-256 verification.
XRAY_VERSION="${XRAY_VERSION:-26.11.9}"
SING_BOX_VERSION="${SING_BOX_VERSION:-1.14.2}"

ACTION="${1:-ensure}"
case "$(uname -m)" in
  x86_64)  ARCH_SB="amd64"; ARCH_XRAY="64" ;;
  aarch64) ARCH_SB="arm64"; ARCH_XRAY="arm64-v8a" ;;
  *) echo "不支持的架构: $(uname -m)" >&2; exit 2 ;;
esac

at_least() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

github_asset_sha256() {
  local repo=$1 tag=$2 asset=$3
  curl -fsSL --proto '=https' --retry 3 \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/${repo}/releases/tags/${tag}" \
    | jq -r --arg name "$asset" '.assets[] | select(.name == $name) | .digest // empty' \
    | sed -n 's/^sha256://p'
}

install_xray() {
  local work asset url expected
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN
  asset="Xray-linux-${ARCH_XRAY}.zip"
  url="https://github.com/XTLS/Xray-core/releases/download/v${XRAY_VERSION}/${asset}"
  expected=$(github_asset_sha256 "XTLS/Xray-core" "v${XRAY_VERSION}" "$asset")
  if [[ -z "$expected" ]]; then
    curl -fsSL --proto '=https' --retry 3 -o "$work/${asset}.dgst" "${url}.dgst"
    expected=$(awk '/SHA2-256/{print $2; exit}' "$work/${asset}.dgst")
  fi
  [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || { echo "无法获取 Xray 官方 SHA-256 摘要，中止" >&2; exit 2; }
  echo "正在下载 Xray v${XRAY_VERSION} ..."
  curl -fsSL --proto '=https' --retry 3 -o "$work/$asset" "$url"
  printf '%s  %s\n' "$expected" "$work/$asset" | sha256sum --check --status
  unzip -p "$work/$asset" xray > "$work/xray"
  chmod 0755 "$work/xray"
  mv -f "$work/xray" /usr/local/bin/xray
  echo "Xray v${XRAY_VERSION} 安装完成"
}

install_singbox() {
  local work asset url expected
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN
  asset="sing-box-${SING_BOX_VERSION}-linux-${ARCH_SB}.tar.gz"
  url="https://github.com/SagerNet/sing-box/releases/download/v${SING_BOX_VERSION}/${asset}"
  expected=$(github_asset_sha256 "SagerNet/sing-box" "v${SING_BOX_VERSION}" "$asset")
  [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || { echo "无法获取 sing-box 官方 SHA-256 摘要，中止" >&2; exit 2; }
  echo "正在下载 sing-box v${SING_BOX_VERSION} ..."
  curl -fsSL --proto '=https' --retry 3 -o "$work/$asset" "$url"
  printf '%s  %s\n' "$expected" "$work/$asset" | sha256sum --check --status
  tar -xzf "$work/$asset" -C "$work" "sing-box-${SING_BOX_VERSION}-linux-${ARCH_SB}/sing-box"
  install -m 0755 "$work/sing-box-${SING_BOX_VERSION}-linux-${ARCH_SB}/sing-box" /usr/local/bin/.sing-box.new
  mv -f /usr/local/bin/.sing-box.new /usr/local/bin/sing-box
  echo "sing-box v${SING_BOX_VERSION} 安装完成"
}

cur_xray=$(/usr/local/bin/xray version 2>/dev/null | awk 'NR==1{print $2}' || true)
cur_sb=$(/usr/local/bin/sing-box version 2>/dev/null | awk 'NR==1{print $3}' || true)

case "$ACTION" in
  ensure)
    if ! at_least "${cur_xray:-0.0.0}" "$XRAY_VERSION"; then install_xray; fi
    if ! at_least "${cur_sb:-0.0.0}" "$SING_BOX_VERSION"; then install_singbox; fi
    ;;
  update)
    install_xray
    install_singbox
    ;;
  *)
    echo "用法: $0 [ensure|update]" >&2
    exit 2
    ;;
esac
