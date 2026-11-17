#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == --help || $# == 0 ]]; then
  echo '用法：sudo bash share-443.sh --node 节点ID [--website-sni 网站域名 --website-port 8443] [--apply]'
  echo '默认只预览。需要已安装的自动 HTTPS 订阅；各服务使用不同 SNI。'
  echo '网站先改为监听 127.0.0.1:8443，证书仍由网站管理。仅透传 TCP TLS，不解密、不写访问日志。'
  echo 'Hysteria2/TUIC 使用 UDP，可直接将其中一个节点设为 UDP 443，与网站 TCP 443 共存。'
  exit 0
fi
command -v sbctl >/dev/null || { echo '请先安装 / 升级 sbctl' >&2; exit 2; }
exec sbctl port share "$@"
