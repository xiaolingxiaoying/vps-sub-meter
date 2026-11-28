#!/usr/bin/env bash
set -euo pipefail

# 纯 Shell acme.sh 证书助手（零 Python 依赖）：支持 Let's Encrypt 签发/续期与 vsm-cert 权限同步，
# 兼容历史 /etc/letsencrypt/live/<domain> 证书同步，并支持无域名模式 10 年 ECDSA P-256 自签证书。

VSM_ROOT="${VSM_ROOT:-/}"
_prefix="${VSM_ROOT%/}"

CERT_BASE="${CERT_BASE:-${_prefix}/var/lib/vps-sub-meter/certificates}"
WEBROOT="${WEBROOT:-${_prefix}/var/lib/vps-sub-meter/acme-webroot}"
ACME_SH_HOME="${ACME_SH_HOME:-${_prefix}/root/.acme.sh}"
ACME_SH="${ACME_SH:-${ACME_SH_HOME}/acme.sh}"
LE_LIVE_BASE="${LE_LIVE_BASE:-${_prefix}/etc/letsencrypt/live}"
LE_RENEWAL_BASE="${LE_RENEWAL_BASE:-${_prefix}/etc/letsencrypt/renewal}"
HOOK_PATH="${HOOK_PATH:-${_prefix}/etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook}"
RENEW_STATUS_FILE="${RENEW_STATUS_FILE:-${_prefix}/var/lib/vps-sub-meter/cert-renew-status.json}"
CONFIG_FILE="${CONFIG_FILE:-${_prefix}/etc/vps-sub-meter/config.toml}"
VSM_BIN="${VSM_BIN:-${_prefix}/usr/local/bin/vps-sub-meter}"
CERT_RENEW_TIMER="vps-sub-meter-cert-renew.timer"

CMD="${1:-}"
shift || true

is_ip_host() {
  local value=${1-}
  [[ -z $value || $value == *:* || $value =~ ^([0-9]+\.){3}[0-9]+$ ]]
}

validate_domain_arg() {
  local domain=${1-}
  if [[ ! $domain =~ ^[[:alnum:]][[:alnum:].-]*$ || $domain == *..* ]]; then
    echo '无效证书名称' >&2
    exit 2
  fi
}

ensure_webroot_dir() {
  mkdir -p "${WEBROOT}/.well-known/acme-challenge"
  chmod 0755 "${WEBROOT}" "${WEBROOT}/.well-known" "${WEBROOT}/.well-known/acme-challenge"
}

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

fix_perms_only() {
  local domain=${1:?missing domain}
  local dest="${CERT_BASE}/${domain}"
  getent group vsm-cert >/dev/null 2>&1 || groupadd --system vsm-cert 2>/dev/null || true
  mkdir -p "$dest"
  chown root:vsm-cert "$dest" 2>/dev/null || true
  chmod 0750 "$dest"
  for f in "${dest}/fullchain.pem" "${dest}/privkey.pem"; do
    if [[ -e "$f" ]]; then
      chown root:vsm-cert "$f" 2>/dev/null || true
      chmod 0640 "$f"
    fi
  done
}

fix_perms_and_reload() {
  local domain=${1:?missing domain}
  fix_perms_only "$domain"
  if [[ "$VSM_ROOT" == "/" ]] && command -v systemctl >/dev/null 2>&1; then
    systemctl try-restart vps-sub-meter.service 2>/dev/null || true
    systemctl try-restart vps-sub-meter-xray.service 2>/dev/null || true
    systemctl try-restart vps-sub-meter-singbox.service 2>/dev/null || true
  fi
}

acme_ecc_args() {
  local domain=${1:?missing domain}
  if [[ -s "${ACME_SH_HOME}/${domain}_ecc/fullchain.cer" || -d "${ACME_SH_HOME}/${domain}_ecc" ]]; then
    echo "--ecc"
  fi
}

acme_cert_dir() {
  local domain=${1:?missing domain}
  if [[ -d "${ACME_SH_HOME}/${domain}_ecc" ]]; then
    printf '%s' "${ACME_SH_HOME}/${domain}_ecc"
  elif [[ -d "${ACME_SH_HOME}/${domain}" ]]; then
    printf '%s' "${ACME_SH_HOME}/${domain}"
  fi
}

acme_cert_file() {
  local domain=${1:?missing domain}
  local dir
  dir=$(acme_cert_dir "$domain")
  if [[ -n "$dir" && -s "${dir}/fullchain.cer" ]]; then
    printf '%s' "${dir}/fullchain.cer"
  fi
}

acme_domain_conf() {
  local domain=${1:?missing domain}
  local dir
  dir=$(acme_cert_dir "$domain")
  if [[ -n "$dir" && -f "${dir}/${domain}.conf" ]]; then
    printf '%s' "${dir}/${domain}.conf"
  fi
}

read_acme_webroot() {
  local conf=${1:?missing conf}
  [[ -r "$conf" ]] || return 1
  awk -F= '
    /^[[:space:]]*Le_Webroot=/ {
      v = substr($0, index($0, "=") + 1)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      if ((v ~ /^'\''.*'\''$/) || (v ~ /^".*"$/)) {
        v = substr(v, 2, length(v) - 2)
      }
      print v
      exit
    }
  ' "$conf"
}

set_acme_webroot() {
  local conf=${1:?missing conf}
  local value=${2:?missing value}
  [[ -f "$conf" && ! -L "$conf" ]] || return 1
  local dir tmp
  dir=$(dirname "$conf")
  tmp=$(mktemp "${dir}/.vsm-acme-conf.XXXXXX")
  awk -v val="$value" '
    BEGIN { updated = 0 }
    /^[[:space:]]*Le_Webroot=/ {
      print "Le_Webroot='\''" val "'\''"
      updated = 1
      next
    }
    { print }
    END {
      if (!updated) {
        print "Le_Webroot='\''" val "'\''"
      }
    }
  ' "$conf" >"$tmp"
  chmod 0600 "$tmp"
  mv -f "$tmp" "$conf"
}

reconcile_acme_webroot() {
  local domain=${1:?missing domain}
  local mode=${2:?missing mode}
  local conf cur target
  conf=$(acme_domain_conf "$domain")
  [[ -n "$conf" && -f "$conf" ]] || return 0
  cur=$(read_acme_webroot "$conf" || true)
  # 仅在 standalone('no')、空或已指向受管 WEBROOT 时切换，绝不覆盖 dns_* 或用户自定义 webroot
  if [[ -n "$cur" && "$cur" != "no" && "$cur" != "$WEBROOT" ]]; then
    return 0
  fi
  if [[ "$mode" == "webroot" ]]; then
    target="$WEBROOT"
  else
    target="no"
  fi
  if [[ "$cur" != "$target" ]]; then
    set_acme_webroot "$conf" "$target" || true
  fi
}

port_80_in_use() {
  ss -tln 2>/dev/null | grep -Eq '(:|\])80([[:space:]]|$)'
}

vsm_serves_webroot() {
  ensure_webroot_dir
  command -v curl >/dev/null 2>&1 || return 1
  local probe="vsm-probe-$$-${RANDOM:-0}"
  local probe_file="${WEBROOT}/.well-known/acme-challenge/${probe}"
  local token="vsm-ok-${probe}"
  printf '%s' "$token" >"$probe_file"
  chmod 0644 "$probe_file"
  local body=""
  body=$(curl --disable --noproxy '*' --silent --max-time 2 \
    "http://127.0.0.1:80/.well-known/acme-challenge/${probe}" 2>/dev/null || true)
  rm -f "$probe_file"
  [[ "$body" == "$token" ]]
}

should_use_webroot() {
  vsm_serves_webroot || port_80_in_use
}

cert_end_epoch() {
  local pem=${1:?missing pem}
  [[ -s "$pem" ]] || return 1
  local end_line end_str epoch
  end_line=$(openssl x509 -in "$pem" -noout -enddate 2>/dev/null || true)
  end_str=${end_line#notAfter=}
  [[ -n "$end_str" ]] || return 1
  epoch=$(date -d "$end_str" +%s 2>/dev/null || true)
  [[ "$epoch" =~ ^-?[0-9]+$ ]] || return 1
  printf '%s' "$epoch"
}

cert_pair_identical() {
  local a=${1:?} b=${2:?}
  [[ -s "$a" && -s "$b" ]] && cmp -s "$a" "$b"
}

install_acme_cert() {
  local domain=${1:?missing domain}
  local dest="${CERT_BASE}/${domain}"
  getent group vsm-cert >/dev/null 2>&1 || groupadd --system vsm-cert 2>/dev/null || true
  mkdir -p "$dest"
  chown root:vsm-cert "$dest" 2>/dev/null || true
  chmod 0750 "$dest"

  local ecc_flag
  ecc_flag=$(acme_ecc_args "$domain")
  local ecc_args=()
  [[ -n "$ecc_flag" ]] && ecc_args=("$ecc_flag")

  "$ACME_SH" --install-cert -d "$domain" "${ecc_args[@]}" \
    --fullchain-file "${dest}/fullchain.pem" \
    --key-file "${dest}/privkey.pem" \
    --reloadcmd "/usr/local/lib/vps-sub-meter/cert.sh reload '${domain}'" >/dev/null
  reconcile_acme_webroot "$domain" "webroot"
  fix_perms_and_reload "$domain"
}

install_certbot_deploy_hook() {
  local domain=${1:?missing domain}
  [[ -d "$LE_RENEWAL_BASE" ]] || return 0
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
  local live="${LE_LIVE_BASE}/${domain}"
  [[ -r "${live}/fullchain.pem" && -r "${live}/privkey.pem" ]] || return 1
  openssl x509 -checkend 1296000 -noout -in "${live}/fullchain.pem" >/dev/null 2>&1
}

sync_legacy_letsencrypt() {
  local domain=${1:?missing domain}
  local live="${LE_LIVE_BASE}/${domain}"
  local dest="${CERT_BASE}/${domain}"
  [[ -r "${live}/fullchain.pem" && -r "${live}/privkey.pem" ]] || return 1
  getent group vsm-cert >/dev/null 2>&1 || groupadd --system vsm-cert 2>/dev/null || true
  mkdir -p "$dest"
  chown root:vsm-cert "$dest" 2>/dev/null || true
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

choose_renew_provider() {
  local domain=${1:?missing domain}
  local has_acme=0 has_certbot=0
  local acme_pem le_pem active_pem
  acme_pem=$(acme_cert_file "$domain" || true)
  le_pem="${LE_LIVE_BASE}/${domain}/fullchain.pem"
  active_pem="${CERT_BASE}/${domain}/fullchain.pem"

  if [[ -x "$ACME_SH" && -n "$(acme_cert_dir "$domain")" ]]; then
    has_acme=1
  fi
  if command -v certbot >/dev/null 2>&1 && [[ -r "$le_pem" ]]; then
    has_certbot=1
  fi

  if ((has_acme && has_certbot)); then
    if cert_pair_identical "$active_pem" "$le_pem" && ! cert_pair_identical "$active_pem" "$acme_pem"; then
      printf 'certbot'
      return 0
    fi
    if [[ -n "$acme_pem" ]] && cert_pair_identical "$active_pem" "$acme_pem"; then
      printf 'acme'
      return 0
    fi
    local acme_end=0 le_end=0
    acme_end=$(cert_end_epoch "$acme_pem" 2>/dev/null || printf '0')
    le_end=$(cert_end_epoch "$le_pem" 2>/dev/null || printf '0')
    if ((le_end > acme_end)); then
      printf 'certbot'
    else
      printf 'acme'
    fi
    return 0
  fi

  if ((has_acme)); then
    printf 'acme'
  elif ((has_certbot)); then
    printf 'certbot'
  else
    printf 'issue'
  fi
}

extract_renew_error() {
  local log_file=${1:?missing log_file}
  local rc=${2:-1}
  local summary=""
  if [[ -s "$log_file" ]]; then
    local cleaned
    cleaned=$(
      sed -E 's/\x1B\[[0-9;]*[[:alpha:]]//g; s/\r//g; s/^\[[^]]+\][[:space:]]*//' "$log_file" \
        | grep -Ev "^(Renewing:|Renewing using Le_API=|Using CA:|Standalone mode\.|Multi-domain=|Getting domain auth token|Please add '--debug'|See: https://github\.com/acmesh-official/acme\.sh|_on_before_issue|_initpath|[[:space:]]*$)" \
        || true
    )
    summary=$(printf '%s\n' "$cleaned" | grep -iE 'error|failed|denied|already used|please stop|timeout|refused|unauthorized|invalid|problem|unable|challenge|rate' | tail -n 2 | tr '\n' ' ' || true)
    if [[ -z "$summary" ]]; then
      summary=$(printf '%s\n' "$cleaned" | tail -n 2 | tr '\n' ' ' || true)
    fi
  fi
  summary=$(printf '%s' "$summary" | sed -E 's/[[:space:]]+/ /g; s/^ //; s/ $//')
  if [[ -z "$summary" ]]; then
    summary="续期命令退出码 ${rc}"
  fi
  if ((${#summary} > 240)); then
    summary="${summary:0:237}..."
  fi
  printf '%s' "$summary"
}

write_renew_status() {
  local domain=${1:?missing domain}
  local status=${2:?missing status}
  local source=${3:?missing source}
  local provider=${4:?missing provider}
  local detail=${5:-}
  local err_msg=${6:-}

  if [[ -L "$RENEW_STATUS_FILE" ]]; then
    echo "拒绝写入符号链接目标: ${RENEW_STATUS_FILE}" >&2
    return 1
  fi
  command -v jq >/dev/null 2>&1 || return 0

  local dir now_ts prev_last_ok="" prev_failures=0 new_failures=0
  dir=$(dirname "$RENEW_STATUS_FILE")
  mkdir -p "$dir"
  now_ts=$(date +%s 2>/dev/null || printf '0')

  if [[ -r "$RENEW_STATUS_FILE" && ! -L "$RENEW_STATUS_FILE" ]]; then
    local prev_domain
    prev_domain=$(jq -r '.domain // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
    if [[ "$prev_domain" == "$domain" ]]; then
      prev_last_ok=$(jq -r 'if .last_success_at == null then "" else (.last_success_at | tostring) end' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      prev_failures=$(jq -r '.consecutive_failures // 0' "$RENEW_STATUS_FILE" 2>/dev/null || printf '0')
      [[ "$prev_failures" =~ ^[0-9]+$ ]] || prev_failures=0
    fi
  fi

  local last_ok_json="null"
  if [[ "$status" == "ok" ]]; then
    last_ok_json="$now_ts"
    new_failures=0
    err_msg=""
  else
    if [[ "$prev_last_ok" =~ ^[0-9]+$ ]]; then
      last_ok_json="$prev_last_ok"
    fi
    new_failures=$((prev_failures + 1))
  fi

  local tmp
  tmp=$(mktemp "${dir}/.vsm-cert-status.XXXXXX")
  jq -n \
    --arg domain "$domain" \
    --arg status "$status" \
    --arg source "$source" \
    --arg provider "$provider" \
    --argjson attempted_at "$now_ts" \
    --argjson last_success_at "$last_ok_json" \
    --argjson consecutive_failures "$new_failures" \
    --arg detail "$detail" \
    --arg error "$err_msg" \
    '{
      domain: $domain,
      status: $status,
      source: $source,
      provider: $provider,
      attempted_at: $attempted_at,
      last_success_at: $last_success_at,
      consecutive_failures: $consecutive_failures,
      detail: (if $detail == "" then null else $detail end),
      error: (if $error == "" then null else $error end)
    }' >"$tmp"
  chmod 0644 "$tmp"
  chown root:vps-sub-meter "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$RENEW_STATUS_FILE"
}

run_acme_issue() {
  local domain=${1:?missing domain}
  local email=${2:-}
  ensure_webroot_dir
  ensure_acme_sh "$email"
  local mode_args=(--standalone)
  if should_use_webroot; then
    mode_args=(--webroot "$WEBROOT")
  fi
  local rc=0
  (
    umask 022
    "$ACME_SH" --issue -d "$domain" "${mode_args[@]}" --keylength ec-256 --server letsencrypt
  ) || rc=$?
  # acme.sh 退出码 2 表示证书尚未到期、无需重签
  if [[ "$rc" -ne 0 && "$rc" -ne 2 ]]; then
    return "$rc"
  fi
  install_acme_cert "$domain"
}

renew_domain() {
  local domain=${1:?missing domain}
  local source=${2:-manual}
  local provider log_file rc=0
  provider=$(choose_renew_provider "$domain")
  log_file=$(mktemp)

  case "$provider" in
    acme)
      ensure_webroot_dir
      if should_use_webroot; then
        reconcile_acme_webroot "$domain" "webroot"
      else
        reconcile_acme_webroot "$domain" "standalone"
      fi
      local ecc_flag ecc_args=()
      ecc_flag=$(acme_ecc_args "$domain")
      [[ -n "$ecc_flag" ]] && ecc_args=("$ecc_flag")
      (
        umask 022
        "$ACME_SH" --renew -d "$domain" "${ecc_args[@]}"
      ) >"$log_file" 2>&1 || rc=$?
      cat "$log_file"
      if [[ "$rc" -ne 0 && "$rc" -ne 2 ]]; then
        local err_msg
        err_msg=$(extract_renew_error "$log_file" "$rc")
        rm -f "$log_file"
        write_renew_status "$domain" "failed" "$source" "$provider" "" "$err_msg" || true
        return "$rc"
      fi
      if [[ "$rc" -eq 2 ]]; then
        local acme_pem active_pem
        acme_pem=$(acme_cert_file "$domain" || true)
        active_pem="${CERT_BASE}/${domain}/fullchain.pem"
        if [[ -n "$acme_pem" ]] && cert_pair_identical "$active_pem" "$acme_pem"; then
          reconcile_acme_webroot "$domain" "webroot"
          fix_perms_only "$domain"
          rm -f "$log_file"
          write_renew_status "$domain" "ok" "$source" "$provider" "证书尚未到期，无需重签" "" || true
          return 0
        fi
      fi
      if ! install_acme_cert "$domain" >"$log_file" 2>&1; then
        local install_rc=$?
        cat "$log_file"
        local err_msg
        err_msg=$(extract_renew_error "$log_file" "$install_rc")
        rm -f "$log_file"
        write_renew_status "$domain" "failed" "$source" "$provider" "" "$err_msg" || true
        return "$install_rc"
      fi
      cat "$log_file"
      rm -f "$log_file"
      write_renew_status "$domain" "ok" "$source" "$provider" "证书已续期并安装" "" || true
      ;;
    certbot)
      ensure_webroot_dir
      local cb_args=(renew --cert-name "$domain" --non-interactive --no-random-sleep-on-renew)
      local le_conf="${LE_RENEWAL_BASE}/${domain}.conf"
      if should_use_webroot && [[ -r "$le_conf" ]] && grep -Eq '^[[:space:]]*authenticator[[:space:]]*=[[:space:]]*standalone([[:space:]]|$)' "$le_conf"; then
        cb_args+=(--authenticator webroot --webroot-path "$WEBROOT")
      fi
      (
        umask 022
        certbot "${cb_args[@]}"
      ) >"$log_file" 2>&1 || rc=$?
      cat "$log_file"
      if [[ "$rc" -ne 0 ]]; then
        local err_msg
        err_msg=$(extract_renew_error "$log_file" "$rc")
        rm -f "$log_file"
        write_renew_status "$domain" "failed" "$source" "$provider" "" "$err_msg" || true
        return "$rc"
      fi
      local le_pem="${LE_LIVE_BASE}/${domain}/fullchain.pem"
      local active_pem="${CERT_BASE}/${domain}/fullchain.pem"
      if cert_pair_identical "$active_pem" "$le_pem"; then
        install_certbot_deploy_hook "$domain"
        fix_perms_only "$domain"
        rm -f "$log_file"
        write_renew_status "$domain" "ok" "$source" "$provider" "证书有效且已同步，无需重载" "" || true
      else
        if ! sync_legacy_letsencrypt "$domain"; then
          local sync_rc=$?
          rm -f "$log_file"
          write_renew_status "$domain" "failed" "$source" "$provider" "" "Certbot 续期后同步证书失败" || true
          return "$sync_rc"
        fi
        rm -f "$log_file"
        write_renew_status "$domain" "ok" "$source" "$provider" "Certbot 证书已续期并同步" "" || true
      fi
      ;;
    issue)
      run_acme_issue "$domain" >"$log_file" 2>&1 || rc=$?
      cat "$log_file"
      if [[ "$rc" -ne 0 ]]; then
        local err_msg
        err_msg=$(extract_renew_error "$log_file" "$rc")
        rm -f "$log_file"
        write_renew_status "$domain" "failed" "$source" "acme" "" "$err_msg" || true
        return "$rc"
      fi
      rm -f "$log_file"
      write_renew_status "$domain" "ok" "$source" "acme" "已签发并安装 ACME 证书" "" || true
      ;;
  esac
}

read_configured_domain() {
  local domain=""
  if [[ -x "$VSM_BIN" ]] && command -v jq >/dev/null 2>&1; then
    domain=$("$VSM_BIN" --root "$VSM_ROOT" config show 2>/dev/null | jq -r '.domain // ""' 2>/dev/null || true)
  fi
  if [[ -z "$domain" && -r "$CONFIG_FILE" ]]; then
    domain=$(awk -F'"' '/^[[:space:]]*domain[[:space:]]*=/{print $2; exit}' "$CONFIG_FILE" 2>/dev/null || true)
  fi
  domain=${domain#"${domain%%[![:space:]]*}"}
  domain=${domain%"${domain##*[![:space:]]}"}
  printf '%s' "$domain"
}

format_epoch() {
  local ts=${1:-0}
  if [[ "$ts" =~ ^[0-9]+$ ]] && ((ts > 0)); then
    date -d "@${ts}" '+%Y-%m-%d %H:%M:%S %Z' 2>/dev/null || printf '%s' "$ts"
  else
    printf '—'
  fi
}

print_auto_status() {
  local domain=${1:-}
  if [[ "$VSM_ROOT" == "/" ]] && command -v systemctl >/dev/null 2>&1; then
    local enabled active next_run
    enabled=$(systemctl is-enabled "$CERT_RENEW_TIMER" 2>/dev/null || printf 'disabled')
    active=$(systemctl is-active "$CERT_RENEW_TIMER" 2>/dev/null || printf 'inactive')
    next_run=$(systemctl show -p NextElapseUSecRealtime --value "$CERT_RENEW_TIMER" 2>/dev/null || true)
    if [[ "$active" == "active" ]]; then
      printf '自动续期：已启用 (%s/%s%s)\n' "$enabled" "$active" "${next_run:+ · 下次 $next_run}"
    else
      printf '自动续期：未启用 (%s/%s)\n' "$enabled" "$active"
    fi
  fi

  if [[ -r "$RENEW_STATUS_FILE" && ! -L "$RENEW_STATUS_FILE" ]] && command -v jq >/dev/null 2>&1; then
    local rec_domain rec_status rec_source rec_provider rec_at rec_ok rec_fail rec_detail rec_err
    rec_domain=$(jq -r '.domain // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
    if [[ -z "$domain" || "$rec_domain" == "$domain" ]]; then
      rec_status=$(jq -r '.status // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      rec_source=$(jq -r '.source // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      rec_provider=$(jq -r '.provider // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      rec_at=$(jq -r '.attempted_at // 0' "$RENEW_STATUS_FILE" 2>/dev/null || printf '0')
      rec_ok=$(jq -r '.last_success_at // 0' "$RENEW_STATUS_FILE" 2>/dev/null || printf '0')
      rec_fail=$(jq -r '.consecutive_failures // 0' "$RENEW_STATUS_FILE" 2>/dev/null || printf '0')
      rec_detail=$(jq -r '.detail // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      rec_err=$(jq -r '.error // ""' "$RENEW_STATUS_FILE" 2>/dev/null || true)
      if [[ "$rec_status" == "failed" ]]; then
        printf '最近续期：失败（%s/%s · %s · 连续失败 %s 次）\n' \
          "$rec_source" "$rec_provider" "$(format_epoch "$rec_at")" "$rec_fail"
        [[ -n "$rec_err" ]] && printf '失败原因：%s\n' "$rec_err"
      elif [[ "$rec_status" == "ok" ]]; then
        printf '最近续期：正常（%s/%s · %s%s）\n' \
          "$rec_source" "$rec_provider" "$(format_epoch "${rec_ok:-$rec_at}")" "${rec_detail:+ · $rec_detail}"
      fi
    fi
  fi
}

case "$CMD" in
  status)
    cert_root=/
    if [[ ${1:-} == --root ]]; then cert_root=${2:?missing root}; shift 2; fi
    domain=${1:?usage: cert.sh status [--root dir] <domain>}
    validate_domain_arg "$domain"
    VSM_ROOT="$cert_root"
    _prefix="${VSM_ROOT%/}"
    CERT_BASE="${_prefix}/var/lib/vps-sub-meter/certificates"
    RENEW_STATUS_FILE="${_prefix}/var/lib/vps-sub-meter/cert-renew-status.json"
    cert=${CERT_BASE}/$domain/fullchain.pem
    if [[ ! -s $cert ]]; then
      printf '未找到当前证书。\n'
      print_auto_status "$domain"
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
    print_auto_status "$domain"
    ;;
  issue)
    domain=${1:?usage: cert.sh issue <domain> [email]}
    validate_domain_arg "$domain"
    email=${2:-}
    if has_valid_letsencrypt_cert "$domain"; then
      echo "检测到现有有效的 Let's Encrypt 证书，直接同步并复用。"
      sync_legacy_letsencrypt "$domain"
      write_renew_status "$domain" "ok" "manual" "certbot" "已复用现有 Let's Encrypt 证书" "" || true
    else
      run_acme_issue "$domain" "$email"
      write_renew_status "$domain" "ok" "manual" "acme" "已签发并安装 ACME 证书" "" || true
    fi
    ;;
  reload)
    domain=${1:?usage: cert.sh reload <domain>}
    validate_domain_arg "$domain"
    fix_perms_and_reload "$domain"
    ;;
  sync)
    domain=${1:?usage: cert.sh sync <domain>}
    validate_domain_arg "$domain"
    acme_pem=$(acme_cert_file "$domain" || true)
    le_pem="${LE_LIVE_BASE}/${domain}/fullchain.pem"
    has_acme_pem=0
    has_le_pem=0
    [[ -n "$acme_pem" && -s "$acme_pem" ]] && has_acme_pem=1
    [[ -r "$le_pem" && -r "${LE_LIVE_BASE}/${domain}/privkey.pem" ]] && has_le_pem=1

    if ((has_acme_pem && has_le_pem)); then
      acme_end=$(cert_end_epoch "$acme_pem" 2>/dev/null || printf '0')
      le_end=$(cert_end_epoch "$le_pem" 2>/dev/null || printf '0')
      if ((le_end > acme_end)); then
        sync_legacy_letsencrypt "$domain"
        write_renew_status "$domain" "ok" "manual" "certbot" "已同步较新的 Let's Encrypt 证书" "" || true
      else
        install_acme_cert "$domain"
        write_renew_status "$domain" "ok" "manual" "acme" "已同步 ACME 证书" "" || true
      fi
    elif ((has_acme_pem)); then
      install_acme_cert "$domain"
      write_renew_status "$domain" "ok" "manual" "acme" "已同步 ACME 证书" "" || true
    elif ((has_le_pem)) && sync_legacy_letsencrypt "$domain"; then
      write_renew_status "$domain" "ok" "manual" "certbot" "已同步 Let's Encrypt 证书" "" || true
    else
      echo "未找到证书: ${domain}" >&2
      exit 2
    fi
    ;;
  renew)
    domain=${1:?usage: cert.sh renew <domain>}
    validate_domain_arg "$domain"
    renew_domain "$domain" "manual"
    ;;
  auto-renew)
    domain=$(read_configured_domain)
    if is_ip_host "$domain"; then
      echo "当前为 IP 模式或未配置域名，跳过自动续期。"
      exit 0
    fi
    validate_domain_arg "$domain"
    renew_domain "$domain" "auto"
    ;;
  auto-status)
    domain=$(read_configured_domain)
    print_auto_status "$domain"
    ;;
  auto-enable)
    command -v systemctl >/dev/null 2>&1 || { echo "系统不支持 systemctl" >&2; exit 2; }
    if [[ ! -f "${_prefix}/etc/systemd/system/${CERT_RENEW_TIMER}" ]]; then
      echo "未找到 ${CERT_RENEW_TIMER}，请先运行 vsm apply 部署服务单元" >&2
      exit 2
    fi
    systemctl daemon-reload
    systemctl enable --now "$CERT_RENEW_TIMER"
    echo "已启用证书自动续期 (${CERT_RENEW_TIMER})"
    ;;
  auto-disable)
    command -v systemctl >/dev/null 2>&1 || { echo "系统不支持 systemctl" >&2; exit 2; }
    systemctl disable --now "$CERT_RENEW_TIMER" 2>/dev/null || true
    echo "已停用证书自动续期 (${CERT_RENEW_TIMER})"
    ;;
  self-signed)
    sni=${1:-www.bing.com}
    validate_domain_arg "$sni"
    dest="${CERT_BASE}/${sni}"
    getent group vsm-cert >/dev/null 2>&1 || groupadd --system vsm-cert 2>/dev/null || true
    mkdir -p "$dest"
    chown root:vsm-cert "$dest" 2>/dev/null || true
    chmod 0750 "$dest"
    if [[ ! -s "${dest}/fullchain.pem" || ! -s "${dest}/privkey.pem" ]]; then
      openssl ecparam -genkey -name prime256v1 -noout -out "${dest}/.privkey.tmp" 2>/dev/null
      openssl req -new -x509 -key "${dest}/.privkey.tmp" -days 3650 -subj "/CN=${sni}" \
        -addext "subjectAltName=DNS:${sni}" -out "${dest}/.fullchain.tmp" 2>/dev/null
      chown root:vsm-cert "${dest}/.privkey.tmp" "${dest}/.fullchain.tmp" 2>/dev/null || true
      chmod 0640 "${dest}/.privkey.tmp" "${dest}/.fullchain.tmp"
      mv -f "${dest}/.privkey.tmp" "${dest}/privkey.pem"
      mv -f "${dest}/.fullchain.tmp" "${dest}/fullchain.pem"
    fi
    ;;
  *)
    echo "用法: $0 {status|issue|reload|sync|renew|auto-renew|auto-status|auto-enable|auto-disable|self-signed} ..." >&2
    exit 2
    ;;
esac
