#!/usr/bin/env bash
set -euo pipefail

# Replaced by prepare-installer.py in the authenticated release packaging job.
# A raw checkout is deliberately not an installable production trust anchor.
SBCTL_PUBLIC_KEY_PEM='@SBCTL_RELEASE_PUBLIC_KEY_PEM@'
if [[ "$SBCTL_PUBLIC_KEY_PEM" == @* ]]; then
  echo "此安装脚本尚未配置生产公钥；请使用发布工件 install.sh。" >&2
  exit 2
fi

# sbctl bootstrap installer.
#
# Download the release installer from:
# https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh
#
# The only trust decisions this script makes are the sbctl binary download,
# which it protects by verifying the Ed25519 signature over the canonical JSON
# of the release manifest BEFORE trusting any URL or digest. Every later trust
# decision (sing-box download, digest, compatibility matrix, candidate checks)
# is made by the installed sbctl binary with the same built-in public key, so
# the script cannot bypass the Rust verification rules.
#
# Deployment writes wait for explicit installation intent. Default bootstrap
# installs management only and leaves the data plane and its credentials intact.

red()   { echo -e "\033[31m\033[01m$*\033[0m"; }
green() { echo -e "\033[32m\033[01m$*\033[0m"; }
yellow(){ echo -e "\033[33m\033[01m$*\033[0m"; }
blue()  { echo -e "\033[36m\033[01m$*\033[0m"; }
white() { echo -e "\033[37m\033[01m$*\033[0m"; }

# GitHub Actions and piped installs may not provide a terminal or TERM.
# Keep the banner useful in those environments without making clear(1) fatal.
if [[ -t 1 && -n "${TERM:-}" && "$TERM" != dumb ]]; then
  clear || true
fi
white "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"
blue  "   sbctl  ·  私有 sing-box 订阅控制面"
white "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"
blue  " 项目  : github.com/xiaolingxiaoying/vps-sub-meter"
blue  "快捷方式: ly"
white "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"
echo

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

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates curl jq openssl tar certbot

# The first-release Ed25519 verification key, identical to the one embedded in
# src/release.rs. The signature in the manifest covers the canonical JSON of
# every field except `signature`, produced exactly like `jq -S -c 'del(.signature)'`.

default_manifest_url='https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/manifest-{arch}.json'
manifest_url_template=${SBCTL_MANIFEST_URL:-$default_manifest_url}
case "$(dpkg --print-architecture)" in
  amd64|arm64) arch=$(dpkg --print-architecture) ;;
  *) echo "仅支持 amd64 和 arm64" >&2; exit 2 ;;
esac
manifest_url=$(printf '%s' "$manifest_url_template" | sed "s/{arch}/$arch/g")
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

# `exec` would replace this shell and skip the EXIT trap above, leaving the
# verified manifest and the downloaded binary behind in /tmp. Run the installer
# and forward its status instead.
run_installer() {
  local status=0
  "$@" || status=$?
  exit "$status"
}

# Prompts read the terminal the operator actually typed on: stdin when it is a
# terminal, otherwise /dev/tty. Both the replacement question and the guided
# argument collection use it, so it is resolved once.
installer_input=
resolve_installer_input() {
  if [[ -n "$installer_input" ]]; then
    return
  fi
  if [[ -t 0 ]]; then
    installer_input=/dev/stdin
    return
  fi
  # `-r /dev/tty` only checks the device node's mode, and the node is readable
  # even for a process with no controlling terminal. Probe by opening it:
  # otherwise a piped install (`curl ... | bash`) on a host with an existing
  # deployment blocks forever on a prompt nobody can answer.
  if { : </dev/tty; } 2>/dev/null; then
    installer_input=/dev/tty
    return
  fi
  echo "检测到已有部署或需要交互输入；请在 VPS 交互终端运行安装脚本。" >&2
  exit 2
}

curl --fail --globoff --location --silent --show-error "$manifest_url" >"$work_dir/manifest.json"

# Verify the manifest signature BEFORE trusting any URL or digest in it. A
# signature failure stops the install with no artifact accessed.
# jq appends a trailing newline, while Rust's canonical JSON signer covers the
# exact compact JSON bytes without one. Remove only jq's record terminator so
# both verification implementations sign and verify the same payload.
jq -S -c 'del(.signature)' "$work_dir/manifest.json" | tr -d '\r\n' >"$work_dir/canonical.json"
jq -r '.signature' "$work_dir/manifest.json" | base64 -d >"$work_dir/signature.bin"
printf '%s\n' "$SBCTL_PUBLIC_KEY_PEM" >"$work_dir/public-key.pem"
if ! openssl pkeyutl -verify -pubin -inkey "$work_dir/public-key.pem" -rawin \
  -in "$work_dir/canonical.json" -sigfile "$work_dir/signature.bin" >/dev/null 2>&1; then
  echo "release manifest 签名校验失败，已中止安装（未访问其中任何下载地址）。" >&2
  exit 2
fi

# The manifest is now trusted: check the schema the same way `sbctl` does, then
# fetch the pinned sbctl and check its digest.
schema=$(jq -er '.schema' "$work_dir/manifest.json")
if [[ "$schema" != 1 ]]; then
  echo "release manifest schema 为 $schema，本安装器只支持 1，已中止安装。" >&2
  exit 2
fi
for field in .sbctl.version .sing_box.version; do
  pinned=$(jq -er "$field" "$work_dir/manifest.json")
  # Same rule as src/release.rs: a floating reference is unsignable in any
  # meaningful sense, and a digest pinned to a moving tag protects nobody.
  if [[ ! "$pinned" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "release manifest 的 $field 不是固定版本号（$pinned），已中止安装。" >&2
    exit 2
  fi
done
artifact_url=$(jq -er '.sbctl.url' "$work_dir/manifest.json")
expected_sha=$(jq -er '.sbctl.sha256' "$work_dir/manifest.json")
# Floating, unsignable references are rejected exactly like the Rust verifier
# rejects "latest"/"main"/"master" version fields: the artifact path must name
# a fixed version, never a moving branch or the GitHub "latest" redirect.
case "$artifact_url" in
  *"/releases/latest/"* | *"/releases/download/latest/"* | *"/latest/download/"* | */latest | */main | */master)
    echo "release manifest 使用了不受支持的 latest/main 引用，已中止安装。" >&2
    exit 2 ;;
esac

# Download and digest-check the candidate into the work directory. The host is
# still untouched: the management binary must not change before the operator
# has decided what to do about an existing deployment.
curl --fail --location --silent --show-error "$artifact_url" >"$work_dir/sbctl"
printf '%s  %s\n' "$expected_sha" "$work_dir/sbctl" | sha256sum --check --status
chmod 0755 "$work_dir/sbctl"

# The read-only preflight runs the *candidate*, never an already installed
# binary: `sbctl install` treats a non-terminal stdin as a preflight, so this
# cannot start an installation or change deployment state.
replace_existing=0
if [[ "$#" -eq 0 ]] || preflight_output=$("$work_dir/sbctl" install </dev/null 2>&1); then
  :
else
  printf '%s\n' "$preflight_output" >&2
  if [[ "$preflight_output" != *"Existing deployment detected"* ]]; then
    exit 2
  fi

  resolve_installer_input
  echo ""
  echo "发现已有 sing-box/sbctl 部署。如何处理？"
  echo "1) 保留现有部署并退出（默认）"
  echo "2) 备份旧部署、停止相关服务、清理冲突路径，然后继续全新安装"
  while :; do
    read -r -p "请选择 [1]: " replace_choice <"$installer_input"
    case "${replace_choice:-1}" in
      1)
        echo "已取消；现有部署未更改（含 /usr/local/bin/sbctl，本次没有替换它）。"
        echo "如需升级已由 sbctl 管理的部署，请运行：sbctl update"
        echo "如需管理现有部署，请运行：ly（或 sbctl menu）"
        exit 0 ;;
      2) break ;;
      *) echo "请输入 1 或 2。" >&2 ;;
    esac
  done
  read -r -p "此操作会重建订阅和协议凭据；输入 REINSTALL 确认: " confirmation <"$installer_input"
  if [[ "$confirmation" != REINSTALL ]]; then
    echo "确认文字不匹配，已取消；现有部署未更改（含 /usr/local/bin/sbctl）。"
    exit 0
  fi
  replace_existing=1
fi

# Default bootstrap installs management only; explicit flags deploy through sbctl.
install_args=("$@")
if [[ "$replace_existing" -eq 1 ]]; then
  install_args+=(--replace-existing)
fi

# Only now is the host changed. Replace by rename, never in place:
# `install`/`cp` truncates the target, and a running sbctl is exactly the case
# during a re-install or an upgrade - writing over a live executable fails with
# ETXTBSY. A rename swaps the directory entry, the old inode stays alive for
# the running process, and the new one is what every later exec sees. `sbctl`
# itself does the same thing in Rust (src/lifecycle.rs).
install -m 0755 "$work_dir/sbctl" /usr/local/bin/.sbctl.new
mv -f /usr/local/bin/.sbctl.new /usr/local/bin/sbctl
if [[ ! -x /usr/local/bin/sbctl ]]; then
  echo "sbctl 二进制未正确安装到 /usr/local/bin/sbctl；现有部署未更改。" >&2
  exit 2
fi
ln -sf /usr/local/bin/sbctl /usr/local/bin/ly
green "sbctl 已安装；快捷方式：ly"

if [[ "$#" -gt 0 ]]; then
  run_installer /usr/local/bin/sbctl install "${install_args[@]}"
fi
green "运行 ly，在安装与部署菜单中选择内核下载源、版本和部署配置。"
if [[ -t 0 ]] || { : </dev/tty; } 2>/dev/null; then
  resolve_installer_input
  run_installer /usr/local/bin/sbctl menu <"$installer_input"
fi
