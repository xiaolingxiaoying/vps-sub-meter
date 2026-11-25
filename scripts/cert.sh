#!/usr/bin/env bash
set -euo pipefail

# Certificate helper: Let's Encrypt issuance/renewal + atomic sync to vsm-cert group,
# or 10-year ECDSA P-256 self-signed fallback for IP-only / firewall-restricted deployments.

CERT_BASE="/var/lib/vps-sub-meter/certificates"
WEBROOT="/var/lib/vps-sub-meter/acme-webroot"
HOOK_PATH="/etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook"

CMD="${1:-}"
shift || true

sync_cert() {
  local domain=${1:?missing domain}
  local live="/etc/letsencrypt/live/${domain}"
  local dest="${CERT_BASE}/${domain}"
  [[ -r "${live}/fullchain.pem" && -r "${live}/privkey.pem" ]] || {
    echo "未找到证书: ${live}" >&2
    exit 2
  }
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  mkdir -p "$dest"
  chown root:vsm-cert "$dest"
  chmod 0750 "$dest"

  install -m 0640 -o root -g vsm-cert -L "${live}/fullchain.pem" "${dest}/.fullchain.pem.tmp" 2>/dev/null \
    || cat "${live}/fullchain.pem" > "${dest}/.fullchain.pem.tmp"
  install -m 0640 -o root -g vsm-cert -L "${live}/privkey.pem" "${dest}/.privkey.pem.tmp" 2>/dev/null \
    || cat "${live}/privkey.pem" > "${dest}/.privkey.pem.tmp"
  chown root:vsm-cert "${dest}/.fullchain.pem.tmp" "${dest}/.privkey.pem.tmp"
  chmod 0640 "${dest}/.fullchain.pem.tmp" "${dest}/.privkey.pem.tmp"
  mv -f "${dest}/.fullchain.pem.tmp" "${dest}/fullchain.pem"
  mv -f "${dest}/.privkey.pem.tmp" "${dest}/privkey.pem"

  install_deploy_hook "$domain"
  systemctl try-restart vps-sub-meter-xray.service 2>/dev/null || true
  systemctl try-restart vps-sub-meter-singbox.service 2>/dev/null || true
}

install_deploy_hook() {
  local domain=${1:?missing domain}
  mkdir -p "$(dirname "$HOOK_PATH")"
  cat >"$HOOK_PATH" <<EOF
#!/bin/sh
set -eu
/usr/local/lib/vps-sub-meter/cert.sh sync "$domain"
EOF
  chmod 0755 "$HOOK_PATH"
}

case "$CMD" in
  issue)
    domain=${1:?usage: cert.sh issue <domain> [email]}
    email=${2:-}
    mkdir -p "${WEBROOT}/.well-known/acme-challenge"
    chmod 0755 "${WEBROOT}" "${WEBROOT}/.well-known" "${WEBROOT}/.well-known/acme-challenge"
    email_args=(--register-unsafely-without-email)
    if [[ -n "$email" ]]; then
      email_args=(--email "$email")
    fi
    if ss -tln | grep -q ':80\b'; then
      certbot certonly --non-interactive --agree-tos "${email_args[@]}" \
        --webroot -w "$WEBROOT" -d "$domain" --keep-until-expiring
    else
      certbot certonly --non-interactive --agree-tos "${email_args[@]}" \
        --standalone -d "$domain" --keep-until-expiring
    fi
    sync_cert "$domain"
    ;;
  sync)
    sync_cert "${1:?usage: cert.sh sync <domain>}"
    ;;
  renew)
    domain=${1:?usage: cert.sh renew <domain>}
    certbot renew --non-interactive --no-random-sleep-on-renew
    sync_cert "$domain"
    ;;
  self-signed)
    sni=${1:-www.bing.com}
    dest="${CERT_BASE}/${sni}"
    getent group vsm-cert >/dev/null || groupadd --system vsm-cert
    mkdir -p "$dest"
    chown root:vsm-cert "$dest"
    chmod 0750 "$dest"
    if [[ ! -s "${dest}/fullchain.pem" || ! -s "${dest}/privkey.pem" ]]; then
      openssl ecparam -genkey -name prime256v1 -noout -out "${dest}/.privkey.tmp" 2>/dev/null
      openssl req -new -x509 -key "${dest}/.privkey.tmp" -days 3650 -subj "/CN=${sni}" \
        -addext "subjectAltName=DNS:${sni}" -out "${dest}/.fullchain.tmp" 2>/dev/null
      chown root:vsm-cert "${dest}/.privkey.tmp" "${dest}/.fullchain.tmp"
      chmod 0640 "${dest}/.privkey.tmp" "${dest}/.fullchain.tmp"
      mv -f "${dest}/.privkey.tmp" "${dest}/privkey.pem"
      mv -f "${dest}/.fullchain.tmp" "${dest}/fullchain.pem"
    fi
    ;;
  *)
    echo "用法: $0 {issue|sync|renew|self-signed} ..." >&2
    exit 2
    ;;
esac
