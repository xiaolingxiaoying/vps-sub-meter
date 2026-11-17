#!/usr/bin/env bash
# Bring different old/new installations to one protocol catalog without reset.
set -euo pipefail
if [[ "${1:-}" == --help ]]; then
  echo '用法：sudo bash protocols.sh [--apply]'
  echo '预览或补齐 9 种协议，并统一为协议名称；保留 ID、原端口、节点凭据与订阅地址。'
  echo '按客户端能力筛选，因此不同客户端仍可能显示不同数量；新增 Xray 会增加一个数据平面进程。'
  exit 0
fi
[[ $# == 0 || ( $# == 1 && "$1" == --apply ) ]] || { echo '未知参数' >&2; exit 2; }
command -v sbctl >/dev/null || { echo '请先运行官方安装器' >&2; exit 2; }
if ! sbctl node complete --help >/dev/null 2>&1; then
  echo '旧管理程序尚不支持统一协议；请先运行 upgrade.sh。无需重装或重置订阅链接。' >&2
  exit 2
fi
if [[ "${1:-}" != --apply ]]; then exec sbctl node complete; fi
[[ $EUID == 0 ]] || { echo '请使用 sudo bash protocols.sh --apply' >&2; exit 2; }
# Keep newer installed cores. Comparing an exact version would downgrade them.
at_least() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}
sing_version=$(/usr/local/bin/sing-box version 2>/dev/null | sed -n '1s/^sing-box version \([^ ]*\).*$/\1/p' || true)
if ! at_least "$sing_version" 1.14.2; then
  sbctl core install sing-box --version 1.14.2
fi
xray_version=$(/usr/local/bin/sbctl-xray version 2>/dev/null | sed -n '1s/^Xray \([^ ]*\).*$/\1/p' || true)
if ! at_least "$xray_version" 26.10.31; then
  sbctl core install xray --version 26.10.31
fi
sbctl node complete --apply
sbctl port status
