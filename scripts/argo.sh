#!/usr/bin/env bash
set -euo pipefail

# Optional Cloudflare Tunnel (`cloudflared`) setup over strict HTTP/2 TCP.

CMD="${1:-}"
shift || true
ARGO_DIR=/etc/cloudflared
TOKEN_FILE=$ARGO_DIR/vps-sub-meter-argo.token
INGRESS_FILE=$ARGO_DIR/vps-sub-meter-argo.yml
ARGO_UNIT=vps-sub-meter-argo.service
UNIT_FILE=/etc/systemd/system/$ARGO_UNIT

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

render_ingress() {
  local config=$1 secrets=$2
  jq --slurpfile keys "$secrets" '
    def trimmed: gsub("^\\s+|\\s+$"; "");
    def is_ip_host: length == 0 or contains(":") or test("^([0-9]+\\.){3}[0-9]+$");
    def regex_literal: explode | map(. as $c | if ("\\.^$|?*+()[]{}" | explode | index($c)) == null then [$c] else [92, $c] end) | add | implode;
    def origin_host:
      if . == "0.0.0.0" then "127.0.0.1" elif . == "::" then "[::1]"
      elif contains(":") then "[\(.)]" else . end;
    . as $cfg | .argo as $argo |
    if $argo.enabled | not then error("Argo 尚未启用") else
    {ingress: ([
      {hostname: ($argo.sub_domain | trimmed | ascii_downcase), path: "^/sub(/|$)", service: "http://127.0.0.1:\($argo.loopback_sub_port)"}
    ] + [ $argo.ws_inbounds[] | . as $ws |
      ($keys[0].argo_ws[$ws.id].path // error("缺少 Argo WS 路径，请先运行 vsm apply")) | split("?")[0] as $path |
      if ($path | startswith("/")) | not then error("Argo WS 路径必须以 / 开头") else
        {hostname: ($argo.tunnel_domain | trimmed | ascii_downcase), path: "^\($path | regex_literal)$", service: "\(if $ws.tls then "https" else "http" end)://\($ws.listen | origin_host):\($ws.port)"}
        + if $ws.tls then ($cfg.domain | trimmed | if is_ip_host then "www.bing.com" else . end) as $sni |
          {originRequest: {originServerName: $sni, caPool: "/var/lib/vps-sub-meter/certificates/\($sni)/fullchain.pem"}}
          else {} end
      end
    ] + [{service: "http_status:404"}])} end
  ' <<<"$config"
}

configure_connector() (
  set -e
  umask 077
  local force_restart=$1 candidate_token=${2:-} staging config changed=0
  config=$(/usr/local/bin/vps-sub-meter config show)
  staging=$(mktemp -d "$ARGO_DIR/.vsm-argo.XXXXXX")
  trap 'rm -rf -- "$staging"' EXIT
  render_ingress "$config" /etc/vps-sub-meter/secrets.json >"$staging/ingress.yml"
  /usr/local/bin/cloudflared tunnel --config "$staging/ingress.yml" ingress validate >/dev/null

  cat >"$staging/unit" <<'EOF'
[Unit]
Description=vps-sub-meter Cloudflare Tunnel (HTTP/2 TCP)
After=network-online.target vps-sub-meter.service vps-sub-meter-xray.service vps-sub-meter-singbox.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/cloudflared tunnel --config /etc/cloudflared/vps-sub-meter-argo.yml --no-autoupdate --no-prechecks --protocol http2 --edge-ip-version 4 --metrics 127.0.0.1:20241 run --token-file /etc/cloudflared/vps-sub-meter-argo.token
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
  if [[ -n $candidate_token ]]; then
    mv -fT "$candidate_token" "$TOKEN_FILE"
  fi
  if ! cmp -s "$staging/ingress.yml" "$INGRESS_FILE"; then
    mv -fT "$staging/ingress.yml" "$INGRESS_FILE"
    changed=1
  fi
  if ! cmp -s "$staging/unit" "$UNIT_FILE"; then
    install -m 0644 "$staging/unit" "$UNIT_FILE"
    systemctl daemon-reload
    changed=1
  fi
  if [[ $force_restart == 1 ]]; then
    systemctl enable "$ARGO_UNIT"
    systemctl restart "$ARGO_UNIT"
  elif [[ $changed == 1 ]]; then
    systemctl try-restart "$ARGO_UNIT"
  fi
)

check_connector() {
  local config port url route svc ready_json ready_conns local_code public_code err_msg ok=0
  local ws_id ws_listen ws_port host_check
  config=$(/usr/local/bin/vps-sub-meter config show)
  port=$(jq -er '.argo.loopback_sub_port' <<<"$config")
  url=$(/usr/local/bin/vps-sub-meter sub --json | jq -er '.argo.shadowrocket // empty') || true
  if [[ -z $url ]]; then
    echo 'Argo 订阅地址不可用，请确认已启用 Argo 并运行 vsm apply。' >&2
    return 1
  fi
  route=${url#*://}
  route=/${route#*/}

  svc=$(systemctl is-active "$ARGO_UNIT" 2>/dev/null || true)
  svc=${svc:-unknown}
  printf '连接器：%s\n' "$svc"
  [[ $svc == active ]] || ok=1

  ready_json=$(curl -sS --max-time 5 http://127.0.0.1:20241/ready 2>/dev/null || true)
  if [[ -n $ready_json ]]; then
    ready_conns=$(jq -r '.readyConnections // empty' <<<"$ready_json" 2>/dev/null || true)
    if [[ -n $ready_conns && $ready_conns =~ ^[0-9]+$ ]]; then
      printf '隧道连接：%s 条就绪连接\n' "$ready_conns"
      ((ready_conns > 0)) || ok=1
    else
      printf '隧道连接：未就绪 (%s)\n' "$ready_json"
      ok=1
    fi
  else
    printf '隧道连接：无法访问本机指标端点 (127.0.0.1:20241)\n'
    ok=1
  fi

  local err_file
  err_file=$(mktemp)
  trap 'rm -f -- "$err_file"' RETURN

  local_code=$(curl -sS --max-time 10 --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port$route" 2>"$err_file" || true)
  err_msg=$(tr '\n' ' ' <"$err_file" | sed 's/[[:space:]]*$//')
  if [[ $local_code == 200 ]]; then
    printf '本机订阅回源：HTTP %s\n' "$local_code"
  else
    printf '本机订阅回源：HTTP %s%s\n' "${local_code:-000}" "${err_msg:+ ($err_msg)}"
    ok=1
  fi

  public_code=$(curl -sS --max-time 12 --output /dev/null --write-out '%{http_code}' "$url" 2>"$err_file" || true)
  err_msg=$(tr '\n' ' ' <"$err_file" | sed 's/[[:space:]]*$//')
  if [[ $public_code == 200 ]]; then
    printf '公网 Argo 订阅：HTTP %s\n' "$public_code"
  else
    printf '公网 Argo 订阅：HTTP %s%s\n' "${public_code:-000}" "${err_msg:+ ($err_msg)}"
    ok=1
  fi

  while IFS=$'\t' read -r ws_id ws_listen ws_port; do
    [[ -n $ws_id ]] || continue
    host_check=${ws_listen#[}
    host_check=${host_check%]}
    if [[ $host_check == 0.0.0.0 ]]; then host_check=127.0.0.1; fi
    if [[ $host_check == :: ]]; then host_check=::1; fi
    if timeout 2 bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$host_check" "$ws_port" 2>/dev/null; then
      printf 'WS 入站 [%s]：%s:%s 正常监听\n' "$ws_id" "$ws_listen" "$ws_port"
    else
      printf 'WS 入站 [%s]：%s:%s 未监听\n' "$ws_id" "$ws_listen" "$ws_port"
      ok=1
    fi
  done < <(jq -r '.argo.ws_inbounds[] | [.id, .listen, (.port | tostring)] | @tsv' <<<"$config")

  return "$ok"
}

case "$CMD" in
  setup)
    if [[ ${1:-} == --reuse-token ]]; then
      if [[ ! -s $TOKEN_FILE ]] || [[ -z $(tr -d '[:space:]' <"$TOKEN_FILE") ]]; then
        echo '未找到已保存的 Tunnel Token，请输入新的 Token' >&2
        exit 2
      fi
      install_cloudflared
      configure_connector 1
      echo 'Argo 连接器已使用已保存的 Token 启动。'
      exit 0
    fi
    if [[ ${1:-} == --token-stdin ]]; then
      IFS= read -r token
    else
      token=${1:?usage: argo.sh setup <tunnel-token>}
    fi
    [[ -n $token ]] || { echo 'Cloudflare Tunnel Token 不能为空' >&2; exit 2; }
    install_cloudflared
    mkdir -p "$ARGO_DIR"
    umask 077
    token_tmp=$(mktemp "$ARGO_DIR/.vsm-argo-token.XXXXXX")
    trap 'rm -f -- "$token_tmp"' EXIT
    printf '%s\n' "$token" >"$token_tmp"
    configure_connector 1 "$token_tmp"
    ;;
  refresh)
    if [[ ! -s $TOKEN_FILE ]] || [[ -z $(tr -d '[:space:]' <"$TOKEN_FILE") ]]; then
      if [[ ${1:-} == --require-token ]]; then
        echo '尚未配置 Tunnel Token，请先执行「配置并启用」' >&2
        exit 2
      fi
      exit 0
    fi
    configure_connector 0
    if [[ ${1:-} == --require-token ]]; then
      if systemctl is-active --quiet "$ARGO_UNIT" 2>/dev/null; then
        echo 'Argo 回源规则已同步，连接器运行正常。'
      else
        echo 'Argo 回源规则已同步，但连接器当前未运行（可执行「配置并启用」启动）。'
      fi
    fi
    ;;
  check)
    check_connector
    ;;
  disable)
    systemctl disable --now "$ARGO_UNIT" 2>/dev/null || true
    ;;
  *)
    echo "用法: $0 {setup <token>|setup --token-stdin|setup --reuse-token|refresh [--require-token]|check|disable}" >&2
    exit 2
    ;;
esac
