#!/usr/bin/env bash
# Recovery upgrade entry point for old sbctl versions. No old CLI is executed.
set -euo pipefail
if [[ "${1:-}" == --help ]]; then
  printf '%s\n' '用法：sudo bash upgrade.sh [--check]' '保留订阅链接、节点、凭据、账期和内核；官方安装器验证发布清单签名与程序摘要，再由新程序事务升级。'
  exit 0
fi
if [[ "${1:-}" == --check ]]; then
  [[ -r /etc/sbctl/config.toml ]] || { echo '未找到 sbctl 配置；本次未修改系统。' >&2; exit 2; }
  printf '%s\n' '已找到旧部署。升级不依赖旧版本 CLI。' '将从项目官方 Release 下载安装器，验证发布清单签名与程序摘要；新程序验证配置，失败保留旧部署。'
  exit 0
fi
[[ $# == 0 ]] || { echo '未知参数；使用 --help 查看说明。' >&2; exit 2; }
[[ $EUID == 0 ]] || { echo '请使用 sudo bash upgrade.sh' >&2; exit 2; }
[[ -f /etc/sbctl/config.toml ]] || { echo '未找到部署；请先首次安装。' >&2; exit 2; }
command -v curl >/dev/null || { echo '请先安装 curl 与 ca-certificates。' >&2; exit 2; }
work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT
curl --fail --location --silent --show-error --proto '=https' --proto-redir '=https' \
  --connect-timeout 15 --max-time 120 --max-filesize 1048576 \
  --output "$work_dir/install.sh" \
  https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh
SBCTL_UPGRADE_ONLY=1 bash "$work_dir/install.sh"
