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

initialize() {
  local current host domain ip interface quota reset_day ports candidate email='' mode sni
  local detected default_ip replace=() show=() default_ports=2
  if [[ -e $config_path || -L $config_path ]]; then
    replace=(--replace)
    default_ports=1
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
  ask '订阅域名或公网 IP' "${host:-$default_ip}" || return 1
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
  ask '节点端口方案 (1 沿用配置 / 2 随机)' "$default_ports" || return 1
  case $answer in
    1) ports=keep ;;
    2) ports=random ;;
    *) printf '请输入 1 或 2。\n' >&2; return 1 ;;
  esac
  candidate=$(vsm config prepare --domain="$domain" --public-ip="$ip" --interface="$interface" \
    --monthly-gib="$quota" --reset-day="$reset_day" --ports="$ports") || return 1
  mode=$(jq -r '.mode' <<<"$candidate") || return 1
  sni=$(jq -r '.tls_sni' <<<"$candidate") || return 1
  if [[ $mode != ip ]]; then
    ask 'ACME 邮箱（回车跳过）' '' || return 1
    email=$answer
  fi
  jq -r '"部署模式：\(.mode)  网卡：\(.config.interface)  月额度：\(.config.traffic.monthly_gb) GiB  重置日：\(.config.traffic.reset_day)",
    (.config.nodes[] | "  \(.name): \(.port)")' <<<"$candidate" || return 1
  confirm '确认保存配置并部署' 1 || return 1

  if [[ $root == / ]]; then
    "$script_dir/cores.sh" ensure || return 1
    "$script_dir/net.sh" tune-sysctl || printf '内核参数调优未全部生效，继续部署。\n' >&2
    if [[ $mode == ip ]]; then
      "$script_dir/cert.sh" self-signed "$sni" || return 1
    else
      "$script_dir/cert.sh" issue "$sni" "$email" || return 1
    fi
  fi
  jq -c '.config' <<<"$candidate" | vsm config write "${replace[@]}" || return 1
  vsm apply
}

subscriptions() {
  local links selection=()
  ask '凭据名称（回车使用默认凭据）' '' || return 1
  if [[ -n $answer ]]; then selection=(--token-name "$answer"); fi
  links=$(vsm sub --json "${selection[@]}") || return 1
  jq -r '"直连订阅", (.direct | to_entries[] | "  \(.key): \(.value)"),
    (if .argo then "Argo 订阅", (.argo | to_entries[] | "  \(.key): \(.value)") else empty end)' <<<"$links" || return 1
  ask '显示直连二维码 (1 Clash / 2 sing-box / 3 Shadowrocket / 0 返回)' || return 1
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

if ! command -v jq >/dev/null 2>&1; then
  printf '找不到 jq，请安装后再使用 ly。\n' >&2
  exit 2
fi

while true; do
  printf '\nvps-sub-meter · 管理菜单\n'
  if [[ -e $config_path ]]; then vsm status; fi
  printf '%s\n' '1. 初始化 / 重新配置' '2. 查看订阅与二维码' '3. 管理订阅凭据' \
    '4. 轮换节点密钥' '5. 应用配置' '0. 退出'
  ask '选择操作' || exit 0
  case $answer in
    1) initialize ;;
    2) subscriptions ;;
    3) tokens ;;
    4) if confirm '确认轮换全部节点密钥，客户端需刷新订阅' 2; then vsm keys rotate --yes; fi ;;
    5) vsm apply ;;
    *) printf '无效选择。\n' >&2 ;;
  esac
done
