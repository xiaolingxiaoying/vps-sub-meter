#!/usr/bin/env bash
{
set -euo pipefail

# vps-sub-meter bootstrap & local installer.
# Signature placeholder replaced by prepare-installer.py in signed releases.
VSM_PUBLIC_KEY_PEM='@VSM_RELEASE_PUBLIC_KEY_PEM@'

red()   { echo -e "\033[31m\033[01m$*\033[0m"; }
green() { echo -e "\033[32m\033[01m$*\033[0m"; }
blue()  { echo -e "\033[36m\033[01m$*\033[0m"; }

LOCAL_DIR=""
FORCE_TEMPLATES=0
PASS_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --local-dir) LOCAL_DIR=${2:?missing dir}; shift 2 ;;
    --force-templates) FORCE_TEMPLATES=1; shift ;;
    *) PASS_ARGS+=("$1"); shift ;;
  esac
done

if [[ "${EUID}" -ne 0 ]]; then
  echo "请使用 root 运行此安装脚本" >&2
  exit 2
fi

if [[ ! -r /etc/os-release ]]; then
  echo "无法识别操作系统" >&2
  exit 2
fi
. /etc/os-release
case "${ID}" in
  debian|ubuntu) ;;
  *) echo "仅支持 Debian 或 Ubuntu" >&2; exit 2 ;;
esac

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  ca-certificates curl jq openssl tar unzip certbot nftables >/dev/null

ensure_accounts_and_dirs() {
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  for u in vps-sub-meter xray sing-box; do
    if ! id -u "$u" >/dev/null 2>&1; then
      useradd --system --no-create-home --shell /usr/sbin/nologin "$u"
    fi
    usermod -aG vsm-cert "$u"
  done

  mkdir -p /etc/vps-sub-meter/templates /etc/vps-sub-meter/overrides \
           /etc/vps-sub-meter/xray /etc/vps-sub-meter/sing-box \
           /var/lib/vps-sub-meter/acme-webroot/.well-known/acme-challenge \
           /var/lib/vps-sub-meter/certificates \
           /var/lib/vps-sub-meter/generations \
           /usr/local/lib/vps-sub-meter

  chown root:root /etc/vps-sub-meter
  chmod 0755 /etc/vps-sub-meter
  chown root:vps-sub-meter /etc/vps-sub-meter/templates /etc/vps-sub-meter/overrides
  chmod 0750 /etc/vps-sub-meter/templates /etc/vps-sub-meter/overrides
  chown root:xray /etc/vps-sub-meter/xray
  chmod 0750 /etc/vps-sub-meter/xray
  chown root:sing-box /etc/vps-sub-meter/sing-box
  chmod 0750 /etc/vps-sub-meter/sing-box

  chown root:vps-sub-meter /var/lib/vps-sub-meter /var/lib/vps-sub-meter/generations
  chmod 1775 /var/lib/vps-sub-meter
  chmod 0750 /var/lib/vps-sub-meter/generations
  chown -R vps-sub-meter:vps-sub-meter /var/lib/vps-sub-meter/acme-webroot
  chmod 0755 /var/lib/vps-sub-meter/acme-webroot /var/lib/vps-sub-meter/acme-webroot/.well-known
  chown root:vsm-cert /var/lib/vps-sub-meter/certificates
  chmod 0750 /var/lib/vps-sub-meter/certificates
}

install_from_dir() {
  local src=$1
  install -m 0755 "$src/vps-sub-meter" /usr/local/bin/.vps-sub-meter.new
  mv -f /usr/local/bin/.vps-sub-meter.new /usr/local/bin/vps-sub-meter
  ln -sf /usr/local/bin/vps-sub-meter /usr/local/bin/vsm
  ln -sf /usr/local/bin/vps-sub-meter /usr/local/bin/ly

  for s in cores.sh cert.sh net.sh argo.sh migrate-sbctl.sh migrate_legacy.py upgrade.sh; do
    if [[ -f "$src/scripts/$s" ]]; then
      install -m 0755 "$src/scripts/$s" "/usr/local/lib/vps-sub-meter/$s"
    fi
  done

  for t in mihomo.yaml sing-box.json; do
    if [[ -f "$src/templates/$t" ]]; then
      if [[ "$FORCE_TEMPLATES" -eq 1 || ! -f "/etc/vps-sub-meter/templates/$t" ]]; then
        install -m 0640 -o root -g vps-sub-meter "$src/templates/$t" "/etc/vps-sub-meter/templates/$t"
      fi
    fi
  done
}

ensure_accounts_and_dirs

if [[ -n "$LOCAL_DIR" ]]; then
  install_from_dir "$LOCAL_DIR"
  green "vps-sub-meter 本地安装完成（快捷命令: vsm / ly）"
  exit 0
fi

if [[ "$VSM_PUBLIC_KEY_PEM" == @* ]]; then
  echo "未提供 --local-dir 且此脚本未嵌入发布公钥；请使用 Release 工件 install.sh。" >&2
  exit 2
fi

case "$(dpkg --print-architecture)" in
  amd64|arm64) arch=$(dpkg --print-architecture) ;;
  *) echo "仅支持 amd64 和 arm64" >&2; exit 2 ;;
esac

manifest_url=${VSM_MANIFEST_URL:-"https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/manifest-${arch}.json"}
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

curl --fail --globoff --location --silent --show-error "$manifest_url" >"$work_dir/manifest.json"
jq -S -c 'del(.signature)' "$work_dir/manifest.json" | tr -d '\r\n' >"$work_dir/canonical.json"
jq -r '.signature' "$work_dir/manifest.json" | base64 -d >"$work_dir/signature.bin"
printf '%s\n' "$VSM_PUBLIC_KEY_PEM" >"$work_dir/public-key.pem"

if ! openssl pkeyutl -verify -pubin -inkey "$work_dir/public-key.pem" -rawin \
  -in "$work_dir/canonical.json" -sigfile "$work_dir/signature.bin" >/dev/null 2>&1; then
  echo "release manifest 签名校验失败，已中止安装。" >&2
  exit 2
fi

schema=$(jq -er '.schema' "$work_dir/manifest.json")
[[ "$schema" == 1 ]] || { echo "不支持的 manifest schema: $schema" >&2; exit 2; }

bin_url=$(jq -er '.vps_sub_meter.url' "$work_dir/manifest.json")
bin_sha=$(jq -er '.vps_sub_meter.sha256' "$work_dir/manifest.json")
bundle_url=$(jq -r '.bundle.url // empty' "$work_dir/manifest.json")
bundle_sha=$(jq -r '.bundle.sha256 // empty' "$work_dir/manifest.json")

curl --fail --location --silent --show-error "$bin_url" >"$work_dir/vps-sub-meter"
printf '%s  %s\n' "$bin_sha" "$work_dir/vps-sub-meter" | sha256sum --check --status
chmod 0755 "$work_dir/vps-sub-meter"

if [[ -n "$bundle_url" && -n "$bundle_sha" ]]; then
  curl --fail --location --silent --show-error "$bundle_url" >"$work_dir/bundle.tar.gz"
  printf '%s  %s\n' "$bundle_sha" "$work_dir/bundle.tar.gz" | sha256sum --check --status
  tar -xzf "$work_dir/bundle.tar.gz" -C "$work_dir"
fi

install_from_dir "$work_dir"
green "vps-sub-meter 已安装（快捷命令: vsm / ly）"

if [[ ${#PASS_ARGS[@]} -gt 0 ]]; then
  /usr/local/bin/vps-sub-meter "${PASS_ARGS[@]}"
elif [[ -t 0 ]] || { : </dev/tty; } 2>/dev/null; then
  /usr/local/bin/vps-sub-meter menu </dev/tty
fi
}
