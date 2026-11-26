#!/usr/bin/env bash
set -euo pipefail

# 纯 Shell acme.sh 证书助手（零 Python 依赖）：支持 Let's Encrypt 签发/续期与 vsm-cert 权限同步，
# 兼容历史 /etc/letsencrypt/live/<domain> 证书同步，并支持无域名模式 10 年 ECDSA P-256 自签证书。

CERT_BASE="/var/lib/vps-sub-meter/certificates"
WEBROOT="/var/lib/vps-sub-meter/acme-webroot"
ACME_SH_HOME="${ACME_SH_HOME:-/root/.acme.sh}"
ACME_SH="${ACME_SH_HOME}/acme.sh"
HOOK_PATH="/etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook"

CMD="${1:-}"
shift || true

ensure_acme_sh() {
  local email=${1:-}
  if [[ -x "$ACME_SH" ]]; then
    return 0
  fi
  local work src
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN
  echo "正在安装纯 Shell ACME 客户端 (acme.sh) ..."
  curl -fsSL --proto '=https' --retry 3 \
    -o "$work/acme.tar.gz" \
    "https://github.com/acmesh-official/acme.sh/archive/refs/heads/master.tar.gz"
  tar -xzf "$work/acme.tar.gz" -C "$work"
  src=$(find "$work" -maxdepth 1 -mindepth 1 -type d | head -n1)
  [[ -n "$src" && -f "$src/acme.sh" ]] || { echo "acme.sh 安装包解压失败" >&2; exit 2; }
  local extra=()
  if [[ -n "$email" ]]; then
    extra=(--accountemail "$email")
  fi
  (
    cd "$src"
    sh ./acme.sh --install --home "$ACME_SH_HOME" "${extra[@]}" >/dev/null
  )
  "$ACME_SH" --set-default-ca --server letsencrypt >/dev/null
}

fix_perms_and_reload() {
  local domain=${1:?missing domain}
  local dest="${CERT_BASE}/${domain}"
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  mkdir -p "$dest"
  chown root:vsm-cert "$dest"
  chmod 0750 "$dest"
  for f in "${dest}/fullchain.pem" "${dest}/privkey.pem"; do
    if [[ -e "$f" ]]; then
      chown root:vsm-cert "$f"
      chmod 0640 "$f"
    fi
  done
  if command -v systemctl >/dev/null 2>&1; then
    systemctl try-restart vps-sub-meter.service 2>/dev/null || true
    systemctl try-restart vps-sub-meter-xray.service 2>/dev/null || true
    systemctl try-restart vps-sub-meter-singbox.service 2>/dev/null || true
  fi
}

acme_ecc_args() {
  local domain=${1:?missing domain}
  if [[ -s "${ACME_SH_HOME}/${domain}_ecc/fullchain.cer" ]]; then
    echo "--ecc"
  fi
}

install_acme_cert() {
  local domain=${1:?missing domain}
  local dest="${CERT_BASE}/${domain}"
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  mkdir -p "$dest"
  chown root:vsm-cert "$dest"
  chmod 0750 "$dest"

  local ecc_flag
  ecc_flag=$(acme_ecc_args "$domain")
  local ecc_args=()
  [[ -n "$ecc_flag" ]] && ecc_args=("$ecc_flag")

  "$ACME_SH" --install-cert -d "$domain" "${ecc_args[@]}" \
    --fullchain-file "${dest}/fullchain.pem" \
    --key-file "${dest}/privkey.pem" \
    --reloadcmd "/usr/local/lib/vps-sub-meter/cert.sh reload '${domain}'" >/dev/null
  fix_perms_and_reload "$domain"
}

install_certbot_deploy_hook() {
  local domain=${1:?missing domain}
  [[ -d /etc/letsencrypt/renewal ]] || return 0
  mkdir -p "$(dirname "$HOOK_PATH")"
  cat >"$HOOK_PATH" <<EOF
#!/bin/sh
set -eu
/usr/local/lib/vps-sub-meter/cert.sh sync "$domain"
EOF
  chmod 0755 "$HOOK_PATH"
}

has_valid_letsencrypt_cert() {
  local domain=${1:?missing domain}
  local live="/etc/letsencrypt/live/${domain}"
  [[ -r "${live}/fullchain.pem" && -r "${live}/privkey.pem" ]] || return 1
  openssl x509 -checkend 1296000 -noout -in "${live}/fullchain.pem" >/dev/null 2>&1
}

sync_legacy_letsencrypt() {
  local domain=${1:?missing domain}
  local live="/etc/letsencrypt/live/${domain}"
  local dest="${CERT_BASE}/${domain}"
  [[ -r "${live}/fullchain.pem" && -r "${live}/privkey.pem" ]] || return 1
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  mkdir -p "$dest"
  chown root:vsm-cert "$dest"
  chmod 0750 "$dest"

  install -m 0640 -o root -g vsm-cert -L "${live}/fullchain.pem" "${dest}/.fullchain.pem.tmp" 2>/dev/null \
    || cat "${live}/fullchain.pem" > "${dest}/.fullchain.pem.tmp"
  install -m 0640 -o root -g vsm-cert -L "${live}/privkey.pem" "${dest}/.privkey.pem.tmp" 2>/dev/null \
    || cat "${live}/privkey.pem" > "${dest}/.privkey.pem.tmp"
  mv -f "${dest}/.fullchain.pem.tmp" "${dest}/fullchain.pem"
  mv -f "${dest}/.privkey.pem.tmp" "${dest}/privkey.pem"
  install_certbot_deploy_hook "$domain"
  fix_perms_and_reload "$domain"
}

run_acme_issue() {
  local domain=${1:?missing domain}
  local email=${2:-}
  mkdir -p "${WEBROOT}/.well-known/acme-challenge"
  chmod 0755 "${WEBROOT}" "${WEBROOT}/.well-known" "${WEBROOT}/.well-known/acme-challenge"
  ensure_acme_sh "$email"
  local mode_args=(--standalone)
  if ss -tln 2>/dev/null | grep -q ':80\b'; then
    mode_args=(--webroot "$WEBROOT")
  fi
  local rc=0
  "$ACME_SH" --issue -d "$domain" "${mode_args[@]}" --keylength ec-256 --server letsencrypt || rc=$?
  # acme.sh 退出码 2 表示证书尚未到期、无需重签
  if [[ "$rc" -ne 0 && "$rc" -ne 2 ]]; then
    return "$rc"
  fi
  install_acme_cert "$domain"
}

case "$CMD" in
  status)
    cert_root=/
    if [[ ${1:-} == --root ]]; then cert_root=${2:?missing root}; shift 2; fi
    domain=${1:?usage: cert.sh status [--root dir] <domain>}
    if [[ ! $domain =~ ^[[:alnum:]][[:alnum:].-]*$ || $domain == *..* ]]; then
      echo '无效证书名称' >&2
      exit 2
    fi
    cert=${cert_root%/}$CERT_BASE/$domain/fullchain.pem
    if [[ ! -s $cert ]]; then
      printf '未找到当前证书。\n'
      exit 0
    fi
    openssl x509 -in "$cert" -noout -subject -issuer -startdate -enddate
    if ! openssl x509 -in "$cert" -noout -checkend 0 >/dev/null; then
      printf '状态：已过期\n'
    elif ! openssl x509 -in "$cert" -noout -checkend 2592000 >/dev/null; then
      printf '状态：30 天内到期\n'
    else
      printf '状态：有效\n'
    fi
    ;;
  issue)
    domain=${1:?usage: cert.sh issue <domain> [email]}
    email=${2:-}
    if has_valid_letsencrypt_cert "$domain"; then
      echo "检测到现有有效的 Let's Encrypt 证书，直接同步并复用。"
      sync_legacy_letsencrypt "$domain"
    else
      run_acme_issue "$domain" "$email"
    fi
    ;;
  reload)
    fix_perms_and_reload "${1:?usage: cert.sh reload <domain>}"
    ;;
  sync)
    domain=${1:?usage: cert.sh sync <domain>}
    if [[ -s "${ACME_SH_HOME}/${domain}_ecc/fullchain.cer" || -s "${ACME_SH_HOME}/${domain}/fullchain.cer" ]]; then
      install_acme_cert "$domain"
    elif sync_legacy_letsencrypt "$domain"; then
      :
    else
      echo "未找到证书: ${domain}" >&2
      exit 2
    fi
    ;;
  renew)
    domain=${1:?usage: cert.sh renew <domain>}
    if [[ -x "$ACME_SH" ]] && [[ -d "${ACME_SH_HOME}/${domain}_ecc" || -d "${ACME_SH_HOME}/${domain}" ]]; then
      ecc_flag=$(acme_ecc_args "$domain")
      ecc_args=()
      [[ -n "$ecc_flag" ]] && ecc_args=("$ecc_flag")
      rc=0
      "$ACME_SH" --renew -d "$domain" "${ecc_args[@]}" || rc=$?
      if [[ "$rc" -ne 0 && "$rc" -ne 2 ]]; then
        exit "$rc"
      fi
      install_acme_cert "$domain"
    elif command -v certbot >/dev/null 2>&1 && [[ -r "/etc/letsencrypt/live/${domain}/fullchain.pem" ]]; then
      certbot renew --non-interactive --no-random-sleep-on-renew
      sync_legacy_letsencrypt "$domain"
    else
      run_acme_issue "$domain"
    fi
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
    echo "用法: $0 {status|issue|reload|sync|renew|self-signed} ..." >&2
    exit 2
    ;;
esac
