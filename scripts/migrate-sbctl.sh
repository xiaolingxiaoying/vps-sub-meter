#!/usr/bin/env bash
set -euo pipefail

# Helper for safely retiring legacy sbctl / standalone PQC units after
# `/etc/vps-sub-meter/` has been populated and validated.

BACKUP="/root/pre-vsm-v2-backup-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
TARGETS=()
for p in /etc/sbctl /var/lib/sbctl /usr/local/etc/xray /usr/local/etc/sing-box-pqc \
         /usr/local/etc/xray-sub /var/lib/xray-sub /etc/*argo-sub /etc/letsencrypt/renewal; do
  [[ -e "$p" ]] && TARGETS+=("$p")
done

if [[ ${#TARGETS[@]} -gt 0 ]]; then
  umask 077
  tar -czf "$BACKUP" "${TARGETS[@]}" 2>/dev/null || true
  echo "已备份旧配置到: $BACKUP"
fi

# Migrate Certbot webroot_path references to /var/lib/vps-sub-meter/acme-webroot
if [[ -d /etc/letsencrypt/renewal ]]; then
  for conf in /etc/letsencrypt/renewal/*.conf; do
    [[ -f "$conf" ]] || continue
    sed -i 's|/var/lib/sbctl/acme-webroot|/var/lib/vps-sub-meter/acme-webroot|g' "$conf"
  done
fi

rm -f /etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook \
      /etc/letsencrypt/renewal-hooks/deploy/xray-sub-reload

LEGACY_UNITS=(
  xray.service
  sing-box-pqc.service
  hy2-porthop.service
  xray-sub.service
  sbctl-http.socket
  sbctl.service
  sing-box.service
  sbctl-xray.service
  sbctl-recover.service
  sbctl-accounting-reset.timer
  sbctl-accounting-reset.service
  sbctl-credential-rotate.timer
  sbctl-credential-rotate.service
  cloudflared-sbctl.service
)
while IFS= read -r extra_unit; do
  [[ -n "$extra_unit" ]] && LEGACY_UNITS+=("$extra_unit")
done < <(systemctl list-unit-files 'sbctl*' '*argo-sub*' --no-legend 2>/dev/null | awk '{print $1}' || true)
for u in "${LEGACY_UNITS[@]}"; do
  systemctl disable --now "$u" 2>/dev/null || true
done

nft delete table ip hy2_porthop 2>/dev/null || true
nft delete table inet sbctl_hy2 2>/dev/null || true

rm -f /usr/local/bin/sbctl
echo "旧 sbctl 与临时 PQC 服务已清理完成（宿主机其他防火墙表未改动）。"
