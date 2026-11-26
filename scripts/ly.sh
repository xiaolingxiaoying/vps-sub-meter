#!/usr/bin/env bash
set -uo pipefail

root=/
if [[ $# -eq 2 && $1 == --root && -n $2 ]]; then
  root=$2
elif [[ $# -ne 0 ]]; then
  printf '用法: ly [--root 目录]\n' >&2
  exit 2
fi
prefix=${root%/}
config_path=$prefix/etc/vps-sub-meter/config.toml
script_dir=$prefix/usr/local/lib/vps-sub-meter

vsm() { "$prefix/usr/local/bin/vps-sub-meter" --root "$root" "$@"; }

ask() {
  local label=$1 default=${2-}
  printf '%s' "$label" >&2
  if [[ $# -gt 1 ]]; then printf ' [%s]' "$default" >&2; fi
  printf ': ' >&2
  IFS= read -r answer || return 1
  answer=${answer#"${answer%%[![:space:]]*}"}
  answer=${answer%"${answer##*[![:space:]]}"}
  [[ $answer != 0 ]] || return 1
  answer=${answer:-$default}
}

confirm() {
  while ask "$1 (1 是 / 2 否 / 0 返回)" "$2"; do
    case $answer in
      1) return 0 ;;
      2) return 1 ;;
      *) printf '请输入 1、2 或 0。\n' >&2 ;;
    esac
  done
  return 1
}

ask_tunnel_token() {
  printf 'Cloudflare Tunnel Token (0 返回): ' >&2
  IFS= read -rs answer || return 1
  printf '\n' >&2
  [[ $answer != 0 ]]
}

direct_nodes=(vless_reality_pqc vless_xhttp_pqc anytls_tcp hysteria2_hop tuic_bbr)

ask_node_port() {
  local config=$1 node=$2 name port value
  local -n options=$3
  name=$(jq -er --arg node "$node" '.nodes[$node].name' <<<"$config") || return 1
  port=$(jq -er --arg node "$node" '.nodes[$node].port' <<<"$config") || return 1
  while ask "$name 端口（443 或 1024–65535）" "$port"; do
    if [[ $answer =~ ^[0-9]{1,5}$ ]]; then
      value=$((10#$answer))
      if ((value == 443 || (value >= 1024 && value <= 65535))); then
        options+=("--port=$node=$value")
        return 0
      fi
    fi
    printf '请输入 443 或 1024–65535 范围内的端口。\n' >&2
  done
  return 1
}

initialize() {
  local current host domain ip interface quota reset_day node candidate email='' mode sni
  local acme_port https_port
  local detected default_ip replace=() show=() port_args=() keep_label='使用模板默认端口'
  if [[ -e $config_path || -L $config_path ]]; then
    replace=(--replace)
    keep_label='沿用当前端口'
  else
    show=(--defaults)
  fi
  current=$(vsm config show "${show[@]}") || return 1
  default_ip=$(jq -r '.public_ip' <<<"$current") || return 1
  interface=$(jq -r '.interface' <<<"$current") || return 1
  if [[ $root == / ]]; then
    if [[ -z $default_ip ]]; then
      default_ip=$(curl --disable --noproxy '*' --fail --silent --connect-timeout 3 --max-time 5 https://api.ipify.org) ||
        default_ip=$(curl --disable --noproxy '*' --fail --silent --connect-timeout 3 --max-time 5 https://api64.ipify.org) || default_ip=''
    fi
    if [[ ${#show[@]} -gt 0 ]]; then
      detected=$(ip -4 route show default | awk 'NR == 1 {for (i = 1; i < NF; i++) if ($i == "dev") {print $(i + 1); break}}') || detected=''
      interface=${detected:-$interface}
    fi
  fi
  host=$(jq -r '.domain' <<<"$current") || return 1
  ask '订阅域名（推荐，启用 HTTPS；填公网 IP 则退回明文 HTTP）' "${host:-$default_ip}" || return 1
  host=$answer
  if [[ $host == *:* || $host =~ ^([0-9]+\.){3}[0-9]+$ ]]; then
    domain=''
    ip=$host
  else
    [[ -n $host ]] || { printf '请输入域名或公网 IP。\n' >&2; return 1; }
    domain=$host
    ask '节点使用的公网 IP' "$default_ip" || return 1
    ip=$answer
  fi
  quota=$(jq -r '.traffic.monthly_gb' <<<"$current") || return 1
  ask '每月流量 GiB（不限可输入 0.0，0 返回）' "$quota" || return 1
  quota=$answer
  reset_day=$(jq -r '.traffic.reset_day' <<<"$current") || return 1
  ask '每月重置日 (1-31)' "$reset_day" || return 1
  reset_day=$answer
  ask "节点端口方案 (1 $keep_label / 2 随机 / 3 手动逐项设置)" 1 || return 1
  case $answer in
    1) port_args=(--ports=keep) ;;
    2) port_args=(--ports=random) ;;
    3)
      port_args=(--ports=keep)
      for node in "${direct_nodes[@]}"; do ask_node_port "$current" "$node" port_args || return 1; done
      ;;
    *) printf '请输入 1、2 或 3。\n' >&2; return 1 ;;
  esac
  candidate=$(vsm config prepare --domain="$domain" --public-ip="$ip" --interface="$interface" \
    --monthly-gib="$quota" --reset-day="$reset_day" "${port_args[@]}") || return 1
  mode=$(jq -r '.mode' <<<"$candidate") || return 1
  sni=$(jq -r '.tls_sni' <<<"$candidate") || return 1
  if [[ $mode != ip ]]; then
    ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 1
    email=$answer
  fi
  jq -r '"部署模式：\(.mode)  网卡：\(.config.interface)  月额度：\(.config.traffic.monthly_gb) GiB  重置日：\(.config.traffic.reset_day)",
    (.config.nodes[] | "  \(.name): \(.port)")' <<<"$candidate" || return 1
  if [[ $mode == ip ]]; then
    printf '警告：未配置域名，订阅使用明文 HTTP。请通过可信渠道导入带证书指纹的客户端配置，或绑定域名启用 HTTPS。\n' >&2
  else
    printf '订阅协议：HTTPS (https://%s/sub/...)\n' "$sni"
  fi
  confirm '确认保存配置并部署' 1 || return 1

  if [[ $root == / ]]; then
    "$script_dir/cores.sh" ensure || return 1
    "$script_dir/net.sh" tune-sysctl || printf '内核参数调优未全部生效，继续部署。\n' >&2
    if [[ $mode == ip ]]; then
      "$script_dir/cert.sh" self-signed "$sni" || return 1
    else
      acme_port=$(jq -r '.config.server.acme_port' <<<"$candidate") || return 1
      https_port=$(jq -r '.config.server.https_port' <<<"$candidate") || return 1
      "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
      "$script_dir/cert.sh" issue "$sni" "$email" || return 1
    fi
  fi
  jq -c '.config' <<<"$candidate" | vsm config write "${replace[@]}" || return 1
  vsm apply
}

manage_ports() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    return 1
  fi
  local current node index=1 candidate port_args=(--ports=keep)
  current=$(vsm config show) || return 1
  for node in "${direct_nodes[@]}"; do
    jq -r --arg node "$node" --arg index "$index" \
      '.nodes[$node] | "\($index). \(.name)  \(.port)\(if .enabled then "" else "（已停用）" end)"' <<<"$current" || return 1
    ((index += 1))
  done
  printf '%s\n' '6. 使用模板默认端口' '7. 重新随机分配端口' '0. 返回'
  ask '选择要修改的直连节点或端口方案' 0 || return 0
  case $answer in
    [1-5]) ask_node_port "$current" "${direct_nodes[answer-1]}" port_args || return 0 ;;
    6) port_args=(--ports=defaults) ;;
    7) port_args=(--ports=random) ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
  candidate=$(vsm config prepare "${port_args[@]}") || return 1
  jq -r '.config.nodes[] | "  \(.name): \(.port)"' <<<"$candidate" || return 1
  if jq -e --argjson current "$current" '.config.nodes == $current.nodes' >/dev/null <<<"$candidate"; then
    printf '节点端口未修改。\n'
    return 0
  fi
  confirm '确认应用节点端口并更新订阅' 2 || return 0
  jq -c '.config' <<<"$candidate" | vsm config apply
}

subscriptions() {
  local links selection=()
  ask '凭据名称（回车使用默认凭据）' '' || return 1
  if [[ -n $answer ]]; then selection=(--token-name "$answer"); fi
  links=$(vsm sub --json "${selection[@]}") || return 1
  jq -r '"直连订阅", (.direct | to_entries[] | "  \(.key): \(.value)"),
    (if .argo then "Argo 订阅", (.argo | to_entries[] | "  \(.key): \(.value)") else empty end)' <<<"$links" || return 1
  if jq -e '.argo != null' >/dev/null <<<"$links"; then
    ask '二维码订阅来源 (1 直连 / 2 Argo 隧道 / 0 返回)' 0 || return 0
    case $answer in
      1) ;;
      2) selection+=(--argo) ;;
      *) printf '无效选择。\n' >&2; return 1 ;;
    esac
  fi
  ask '二维码格式 (1 Clash / 2 sing-box / 3 Shadowrocket / 0 返回)' || return 1
  case $answer in
    1) vsm sub --qr "${selection[@]}" ;;
    2) vsm sub --qr-format sing-box "${selection[@]}" ;;
    3) vsm sub --qr-format shadowrocket "${selection[@]}" ;;
    '') return 0 ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

tokens() {
  local name
  vsm token list --json | jq -r '.[] | [.name, .masked, (if .default then "默认" else "" end)] | @tsv' || return 1
  ask '凭据操作 (1 新增 / 2 轮换默认 / 3 吊销 / 0 返回)' || return 1
  case $answer in
    1)
      ask '设备名称' || return 1
      vsm token add -- "$answer"
      ;;
    2)
      confirm '确认轮换默认凭据，旧订阅链接将失效' 2 || return 1
      vsm token rotate
      ;;
    3)
      ask '要吊销的凭据名称' || return 1
      name=$answer
      confirm '确认吊销该凭据' 2 || return 1
      vsm token revoke -- "$name"
      ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

uninstall_vsm() {
  local flags=() rc=0
  if [[ ! -f $script_dir/uninstall.sh ]]; then
    printf '找不到卸载脚本: %s\n' "$script_dir/uninstall.sh" >&2
    return 1
  fi
  ask '卸载模式 (1 彻底卸载 / 2 保留配置与流量账本 / 0 返回)' 0 || return 0
  case $answer in
    1) flags=() ;;
    2) flags=(--keep-config) ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
  bash "$script_dir/uninstall.sh" --root "$root" "${flags[@]}" || rc=$?
  case $rc in
    0) exit 0 ;;
    10) return 0 ;;
    *) return "$rc" ;;
  esac
}

update_cores() {
  if [[ ! -f $script_dir/cores.sh ]]; then
    printf '找不到内核更新脚本: %s\n' "$script_dir/cores.sh" >&2
    return 1
  fi
  confirm '确认更新 Xray 与 sing-box 内核到脚本指定版本' 1 || return 0
  if [[ $root == / ]]; then
    bash "$script_dir/cores.sh" update || return 1
  fi
  if [[ -e $config_path ]]; then
    vsm apply
  fi
}

manage_argo() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    return 1
  fi
  local current enabled td_cur sd_cur token td sd updated
  current=$(vsm config show) || return 1
  enabled=$(jq -r '.argo.enabled' <<<"$current") || return 1
  td_cur=$(jq -r '.argo.tunnel_domain' <<<"$current") || return 1
  sd_cur=$(jq -r '.argo.sub_domain' <<<"$current") || return 1
  printf '当前 Argo 状态: %s' "$([[ $enabled == true ]] && printf '已启用' || printf '未启用')" >&2
  if [[ -n $td_cur ]]; then
    printf ' (tunnel=%s, sub=%s)' "$td_cur" "${sd_cur:-$td_cur}" >&2
  fi
  printf '\n' >&2

  ask 'Argo 隧道操作 (1 配置并启用 / 2 停用 / 3 检查本机与公网订阅 / 4 更新回源规则 / 0 返回)' 0 || return 0
  case $answer in
    1)
      ask_tunnel_token || return 0
      token=$answer
      [[ -n $token ]] || { printf 'Tunnel Token 不能为空。\n' >&2; return 1; }
      ask 'Argo 节点接入域名 (WebSocket)' "$td_cur" || return 0
      td=$answer
      [[ -n $td ]] || { printf 'tunnel_domain 不能为空。\n' >&2; return 1; }
      ask 'Argo 订阅域名 (可与节点域名相同，需指向本隧道)' "${sd_cur:-$td}" || return 0
      sd=${answer:-$td}
      confirm '确认写入 Argo 配置并生效' 1 || return 0
      updated=$(jq -c --arg td "$td" --arg sd "$sd" \
        '.argo.enabled = true | .argo.tunnel_domain = $td | .argo.sub_domain = $sd' <<<"$current") || return 1
      vsm config apply <<<"$updated" || return 1
      if [[ $root == / && -f $script_dir/argo.sh ]]; then
        bash "$script_dir/argo.sh" setup --token-stdin <<<"$token" || return 1
      fi
      ;;
    2)
      confirm '确认停用 Argo 隧道并重新应用配置' 2 || return 0
      updated=$(jq -c '.argo.enabled = false' <<<"$current") || return 1
      vsm config apply <<<"$updated" || return 1
      if [[ $root == / && -f $script_dir/argo.sh ]]; then
        bash "$script_dir/argo.sh" disable || true
      fi
      ;;
    3)
      if [[ $enabled != true ]]; then
        printf 'Argo 隧道尚未启用。\n' >&2
        return 1
      fi
      if [[ $root == / ]]; then bash "$script_dir/argo.sh" check; fi
      ;;
    4)
      [[ $enabled == true ]] || return 1
      confirm '确认重新生成 Cloudflared 回源规则' 2 || return 0
      if [[ $root == / ]]; then bash "$script_dir/argo.sh" refresh; fi
      ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

manage_domain() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    return 1
  fi
  local current cur_domain acme_port https_port new_domain email='' candidate mode sni
  current=$(vsm config show) || return 1
  cur_domain=$(jq -r '.domain' <<<"$current") || return 1
  acme_port=$(jq -r '.server.acme_port' <<<"$current") || return 1
  https_port=$(jq -r '.server.https_port' <<<"$current") || return 1

  if [[ -z $cur_domain ]]; then
    printf '当前状态: 未配置域名（明文 HTTP 模式，存在泄露风险）\n' >&2
  else
    printf '当前域名: %s（HTTPS 加密模式）\n' "$cur_domain" >&2
  fi

  ask '域名与 HTTPS 操作 (1 绑定/更换域名并启用 HTTPS / 2 同步或续期当前证书 / 0 返回)' 1 || return 0
  case $answer in
    1)
      ask '输入已解析到本机的域名（用于启用 HTTPS）' "$cur_domain" || return 0
      new_domain=$answer
      if [[ -z $new_domain || $new_domain == *:* || $new_domain =~ ^([0-9]+\.){3}[0-9]+$ ]]; then
        printf '请输入有效域名（不能为纯 IP），以签发 TLS 证书并启用 HTTPS。\n' >&2
        return 1
      fi
      ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 0
      email=$answer
      candidate=$(vsm config prepare --domain="$new_domain" --ports=keep) || return 1
      mode=$(jq -r '.mode' <<<"$candidate") || return 1
      sni=$(jq -r '.tls_sni' <<<"$candidate") || return 1
      acme_port=$(jq -r '.config.server.acme_port' <<<"$candidate") || return 1
      https_port=$(jq -r '.config.server.https_port' <<<"$candidate") || return 1
      printf '订阅模式：%s  订阅域名：%s  (https://%s/sub/...)\n' "$mode" "$sni" "$sni" >&2
      confirm '确认保存域名配置、签发/同步 TLS 证书并启用 HTTPS' 1 || return 0
      if [[ $root == / ]]; then
        "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
        "$script_dir/cert.sh" issue "$sni" "$email" || return 1
      fi
      jq -c '.config' <<<"$candidate" | vsm config write --replace || return 1
      vsm apply
      ;;
    2)
      if [[ -z $cur_domain ]]; then
        printf '尚未配置域名，请先选择 1 绑定域名。\n' >&2
        return 1
      fi
      if [[ $root == / ]]; then
        "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
        "$script_dir/cert.sh" renew "$cur_domain"
      fi
      ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

manage_certificate() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    return 1
  fi
  local config domain sni email
  config=$(vsm config show) || return 1
  domain=$(jq -r '.domain | gsub("^\\s+|\\s+$"; "")' <<<"$config") || return 1
  sni=${domain:-www.bing.com}
  printf '当前证书名称：%s\n' "$sni"
  bash "$script_dir/cert.sh" status --root "$root" "$sni" || true
  if [[ -n $domain ]]; then
    printf '%s\n' '1. 续期当前域名证书' '2. 同步已有 ACME 证书' '3. 申请或复用 Let’s Encrypt 证书' '0. 返回'
  else
    printf '%s\n' '1. 检查并补齐 IP 模式的自签证书' '0. 返回'
  fi
  ask '选择证书操作' 0 || return 0
  case $answer in
    1)
      if [[ -n $domain ]]; then
        confirm '确认检查并续期证书' 2 || return 0
        if [[ $root == / ]]; then bash "$script_dir/cert.sh" renew "$sni"; fi
      else
        confirm '确认检查并补齐自签证书' 2 || return 0
        if [[ $root == / ]]; then
          bash "$script_dir/cert.sh" self-signed "$sni" || return 1
          vsm apply || return 1
          bash "$script_dir/cert.sh" reload "$sni"
        fi
      fi
      ;;
    2)
      [[ -n $domain ]] || return 1
      confirm '确认同步已有证书' 2 || return 0
      if [[ $root == / ]]; then bash "$script_dir/cert.sh" sync "$sni"; fi
      ;;
    3)
      [[ -n $domain ]] || return 1
      ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 0
      email=$answer
      confirm '确认申请或复用 Let’s Encrypt 证书' 2 || return 0
      if [[ $root == / ]]; then bash "$script_dir/cert.sh" issue "$sni" "$email"; fi
      ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

manage_traffic() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    return 1
  fi
  local current settings quota day time reset_tz display_tz upload download args=()

  printf '%s\n' '1. 修改月流量上限、重置日、时间和时区' \
    '2. 校准已用上传和下载流量' '3. 立即保存当前流量账本' '0. 返回'
  ask '选择流量操作' 0 || return 0
  case $answer in
    1)
      current=$(vsm config show) || return 1
      settings=$(jq -r '.traffic | [.monthly_gb, .reset_day, .reset_time, .reset_tz, .display_tz] | @tsv' <<<"$current") || return 1
      IFS=$'\t' read -r quota day time reset_tz display_tz <<<"$settings"
      ask '月流量上限 (GiB，0.0 表示不限)' "$quota" || return 0
      [[ $answer == "$quota" ]] || args+=("--monthly-gib=$answer")
      ask '每月重置日 (1-31)' "$day" || return 0
      [[ $answer == "$day" ]] || args+=("--reset-day=$answer")
      ask '重置时间 (HH:MM)' "$time" || return 0
      [[ $answer == "$time" ]] || args+=("--reset-time=$answer")
      ask '重置时区 (IANA，如 Asia/Shanghai)' "$reset_tz" || return 0
      [[ $answer == "$reset_tz" ]] || args+=("--reset-tz=$answer")
      ask '显示时区 (IANA，如 Asia/Shanghai)' "$display_tz" || return 0
      [[ $answer == "$display_tz" ]] || args+=("--display-tz=$answer")
      if [[ ${#args[@]} -eq 0 ]]; then
        printf '流量设置未修改\n'
        return 0
      fi
      confirm '确认保存流量设置' 2 || return 0
      vsm traffic configure "${args[@]}"
      ;;
    2)
      ask '已用客户端上传 (GiB，清零请输入 0.0)' || return 0
      upload=$answer
      ask '已用客户端下载 (GiB，清零请输入 0.0)' || return 0
      download=$answer
      confirm '确认覆盖本期已用上传和下载流量' 2 || return 0
      vsm traffic set -- "$upload" "$download"
      ;;
    3)
      confirm '确认立即保存流量账本' 2 || return 0
      vsm traffic checkpoint
      ;;
    *) printf '无效选择。\n' >&2; return 1 ;;
  esac
}

main() {
  if ! command -v jq >/dev/null 2>&1; then
    printf '找不到 jq，请安装后再使用 ly。\n' >&2
    exit 2
  fi

  while true; do
    printf '\nvps-sub-meter · 管理菜单\n'
    if [[ -e $config_path ]]; then vsm status; fi
    printf '%s\n' '1. 初始化 / 重新配置' '2. 查看订阅与二维码' '3. 管理订阅凭据' \
      '4. 轮换节点密钥' '5. 更新内核 (Xray / sing-box)' '6. 配置 / 切换域名 (HTTPS)' \
      '7. 管理 Argo 隧道' '8. 应用配置' '9. 卸载 vps-sub-meter' '10. 流量账期与校准' \
      '11. 节点端口设置' '12. 证书管理' '0. 退出'
    ask '选择操作' || exit 0
    case $answer in
      1) initialize ;;
      2) subscriptions ;;
      3) tokens ;;
      4) if confirm '确认轮换全部节点密钥，客户端需刷新订阅' 2; then vsm keys rotate --yes; fi ;;
      5) update_cores ;;
      6) manage_domain ;;
      7) manage_argo ;;
      8) vsm apply ;;
      9) uninstall_vsm ;;
      10) manage_traffic ;;
      11) manage_ports ;;
      12) manage_certificate ;;
      *) printf '无效选择。\n' >&2 ;;
    esac
  done
}

main
