#!/usr/bin/env bash
set -euo pipefail

# Optional Cloudflare Tunnel (`cloudflared`) setup over strict HTTP/2 TCP.

CMD="${1:-}"
shift || true

install_cloudflared() {
  if [[ -x /usr/local/bin/cloudflared ]]; then
    return 0
  fi
  case "$(dpkg --print-architecture)" in
    amd64) arch="amd64" ;;
    arm64) arch="arm64" ;;
    *) echo "不支持的架构" >&2; exit 2 ;;
  esac
  curl -fsSL --proto '=https' --retry 3 \
    -o /usr/local/bin/.cloudflared.new \
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-${arch}"
  chmod 0755 /usr/local/bin/.cloudflared.new
  mv -f /usr/local/bin/.cloudflared.new /usr/local/bin/cloudflared
}

case "$CMD" in
  setup)
    token=${1:?usage: argo.sh setup <tunnel-token>}
    install_cloudflared
    mkdir -p /etc/cloudflared
    umask 077
    printf '%s\n' "$token" >/etc/cloudflared/vps-sub-meter-argo.token
    chmod 0600 /etc/cloudflared/vps-sub-meter-argo.token

    cat >/etc/systemd/system/vps-sub-meter-argo.service <<'EOF'
[Unit]
Description=vps-sub-meter Cloudflare Tunnel (HTTP/2 TCP)
After=network-online.target vps-sub-meter.service vps-sub-meter-xray.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/cloudflared tunnel --no-autoupdate --no-prechecks --protocol http2 --edge-ip-version 4 --metrics 127.0.0.1:20241 run --token-file /etc/cloudflared/vps-sub-meter-argo.token
Restart=on-failure
RestartSec=5s
TimeoutStopSec=35s
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/etc/cloudflared

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable --now vps-sub-meter-argo.service
    ;;
  disable)
    systemctl disable --now vps-sub-meter-argo.service 2>/dev/null || true
    ;;
  *)
    echo "用法: $0 {setup <token>|disable}" >&2
    exit 2
    ;;
esac
