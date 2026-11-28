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

shopt -s checkwinsize 2>/dev/null || true

interactive=false
color=false
if [[ -t 0 && -t 1 && -t 2 ]]; then
  interactive=true
  if [[ ${TERM:-dumb} != dumb && -z ${NO_COLOR:-} ]]; then color=true; fi
fi

alt_screen_active=false
winch_resized=0
version_bg_pid=''
RAW_TERM_COLS=80
RAW_TERM_LINES=24
TERM_COLS=80
TERM_LINES=24
LAST_RAW_COLS=0
LAST_RAW_LINES=0
VERSION_CACHE_TTL=1800

# ── 缓存的面板展示数据（窗口缩放重绘时复用，避免闪烁与重复查询） ──
VPS_INFO_LOADED=0
VPS_OS='未知'
VPS_KERNEL='未知'
VPS_CPU='未知'
VPS_VIRT='未知'
VPS_BBR='未知'
VPS_IPV4='无'
VPS_IPV6='无'
VPS_PUB_IPV4_CACHE=''

CUR_VSM_VER=''
CUR_XRAY_VER='未安装'
CUR_SB_VER='未安装'
SVC_VSM_STATE='未知'
SVC_XRAY_STATE='未知'
SVC_SB_STATE='未知'
SVC_ARGO_STATE='未启用'

LATEST_VSM_VER=''
LATEST_XRAY_VER=''
LATEST_SB_VER=''
HAS_VSM_UPDATE=0
HAS_XRAY_UPDATE=0
HAS_SB_UPDATE=0
CACHE_MTIME_SEEN=''

DASH_INITIALIZED=0
DASH_STATUS_JSON=''
DASH_CONFIG_JSON=''
DASH_CERT_SUMMARY='未初始化'
DASH_CERT_AUTO_STATE='未知'
DASH_CERT_RENEW_FAILED=0
DASH_CERT_RENEW_ERR=''
DASH_CERT_RENEW_TIME=''
DASH_CERT_RENEW_COUNT=0
DASH_REALITY_SNI='—'
DASH_NODES_LINE1=''
DASH_NODES_LINE2=''
DASH_PANEL_SSH_CMD=''

paint() {
  if "$color"; then
    printf '\033[%sm%s\033[0m' "$1" "$2"
  else
    printf '%s' "$2"
  fi
}

enter_alt_screen() {
  if "$interactive" && [[ ${TERM:-dumb} != dumb ]] && ! "$alt_screen_active"; then
    printf '\033[?1049h\033[?25h'
    alt_screen_active=true
  fi
}

leave_alt_screen() {
  if "$alt_screen_active"; then
    printf '\033[0m\033[?25h\033[?1049l'
    alt_screen_active=false
  fi
}

cleanup() {
  if [[ -n ${version_bg_pid:-} ]]; then
    kill "$version_bg_pid" 2>/dev/null || true
    wait "$version_bg_pid" 2>/dev/null || true
    version_bg_pid=''
  fi
  leave_alt_screen
}

clear_screen() {
  if "$interactive" && [[ ${TERM:-dumb} != dumb ]]; then
    printf '\033[H\033[2J\033[3J'
  fi
}

query_raw_term_size() {
  local size rows='' cols=''
  if "$interactive"; then
    if size=$(stty size </dev/tty 2>/dev/null) && [[ $size =~ ^([0-9]+)[[:space:]]+([0-9]+)$ ]]; then
      rows=${BASH_REMATCH[1]}
      cols=${BASH_REMATCH[2]}
    fi
    if [[ -z $cols ]] && command -v tput >/dev/null 2>&1; then
      rows=$(tput lines 2>/dev/null || true)
      cols=$(tput cols 2>/dev/null || true)
    fi
  fi
  if [[ $rows =~ ^[0-9]+$ && $cols =~ ^[0-9]+$ ]] && ((rows > 0 && cols > 0)); then
    RAW_TERM_LINES=$rows
    RAW_TERM_COLS=$cols
    return 0
  fi
  return 1
}

refresh_term_size() {
  local rows='' cols=''
  if query_raw_term_size; then
    rows=$RAW_TERM_LINES
    cols=$RAW_TERM_COLS
  else
    rows=${LINES:-24}
    cols=${COLUMNS:-80}
    [[ $rows =~ ^[0-9]+$ ]] && ((rows > 0)) || rows=24
    [[ $cols =~ ^[0-9]+$ ]] && ((cols > 0)) || cols=80
    RAW_TERM_LINES=$rows
    RAW_TERM_COLS=$cols
  fi
  ((rows < 16)) && rows=16
  ((cols < 48)) && cols=48
  TERM_LINES=$rows
  TERM_COLS=$cols
}

panel_width() {
  local w=$((TERM_COLS - 2))
  ((w > 116)) && w=116
  ((w < 46)) && w=46
  printf '%d' "$w"
}

vis_width() {
  local LC_ALL=C
  local s=$1 clean='' i=0 n byte width=0
  while [[ $s =~ ^([^$'\033']*)$'\033'\[[0-9\;]*[a-zA-Z](.*)$ ]]; do
    clean+=${BASH_REMATCH[1]}
    s=${BASH_REMATCH[2]}
  done
  clean+=$s
  n=${#clean}
  while ((i < n)); do
    printf -v byte '%d' "'${clean:i:1}"
    if ((byte < 128)); then
      ((width += 1, i += 1))
    elif ((byte < 224)); then
      ((width += 1, i += 2))
    elif ((byte < 240)); then
      if ((byte == 226 && i + 2 < n)); then
        ((width += 1))
      else
        ((width += 2))
      fi
      ((i += 3))
    else
      ((width += 2, i += 4))
    fi
  done
  printf '%d' "$width"
}

repeat_char() {
  local ch=$1 count=$2 out=''
  ((count <= 0)) && return 0
  printf -v out '%*s' "$count" ''
  printf '%s' "${out// /$ch}"
}

pad_right() {
  local text=$1 target=$2 w pad
  w=$(vis_width "$text")
  printf '%s' "$text"
  if ((w < target)); then
    pad=$((target - w))
    printf '%*s' "$pad" ''
  fi
}

hr_line() {
  local ch=${1:-─} style=${2:-2} w
  w=$(panel_width)
  printf ' '
  paint "$style" "$(repeat_char "$ch" "$w")"
  printf '\n'
}

section_bar() {
  local title=$1 w title_w fill
  w=$(panel_width)
  title_w=$(vis_width "$title")
  fill=$((w - title_w - 4))
  ((fill < 2)) && fill=2
  printf ' '
  paint 2 '── '
  paint '1;36' "$title"
  paint 2 " $(repeat_char '─' "$fill")"
  printf '\n'
}

print_kv_pair() {
  local l_key=$1 l_val=$2 r_key=${3-} r_val=${4-}
  local w col_w left_text right_text lw rw
  w=$(panel_width)
  left_text="$(paint 2 "${l_key}:") ${l_val}"
  if [[ -z $r_key ]]; then
    printf '  %s\n' "$left_text"
    return 0
  fi
  right_text="$(paint 2 "${r_key}:") ${r_val}"
  if ((TERM_COLS >= 88)); then
    col_w=$((w / 2))
    lw=$(vis_width "$left_text")
    rw=$(vis_width "$right_text")
    if ((lw + 2 <= col_w && col_w + rw <= w)); then
      printf '  %s%s\n' "$(pad_right "$left_text" "$col_w")" "$right_text"
      return 0
    fi
  fi
  printf '  %s\n' "$left_text"
  printf '  %s\n' "$right_text"
}

menu_heading() {
  if "$interactive"; then
    clear_screen
    refresh_term_size
    hr_line '━' '36'
    printf '  '
    paint '1;36' "$1"
    printf '\n'
    hr_line '─' '2'
  else
    printf '\n'
    paint 2 '------------------------------------------------'
    printf '\n'
    paint '1;36' "$1"
    printf '\n'
  fi
}

menu_section() {
  if "$interactive"; then
    ((TERM_LINES >= 30)) && printf '\n'
    section_bar "$1"
  else
    printf '\n'
    paint '1;36' "$1"
    printf '\n'
  fi
}

menu_item() {
  local number
  printf -v number '%2s.' "$1"
  printf '  '
  paint "${3:-32}" "$number"
  printf ' %s\n' "$2"
}

ask() {
  local label=$1 default=${2-}
  paint '1;33' "$label" >&2
  if [[ -n $default ]]; then printf ' [%s]' "$default" >&2; fi
  printf ': ' >&2
  IFS= read -r answer || return 1
  answer=${answer#"${answer%%[![:space:]]*}"}
  answer=${answer%"${answer##*[![:space:]]}"}
  [[ $answer != 0 ]] || return 1
  answer=${answer:-$default}
}

choose() {
  local last=$1
  shift
  while ask "$@" && [[ $answer != 0 ]]; do
    if [[ $answer =~ ^[1-9][0-9]?$ ]] && ((answer <= last)); then
      return 0
    fi
    printf '请输入 0–%s 之间的编号。\n' "$last" >&2
  done
  return 1
}

confirm() {
  choose 2 "$1（1 是 / 2 否 / 0 返回）" "$2" && [[ $answer == 1 ]]
}

ask_tunnel_token() {
  local prompt=${1:-'Cloudflare Tunnel Token（0 返回）'}
  paint '1;33' "$prompt" >&2
  printf ': ' >&2
  IFS= read -rs answer || return 1
  printf '\n' >&2
  [[ $answer != 0 ]]
}

is_ip_host() {
  local value=${1-}
  [[ -z $value || $value == *:* || $value =~ ^([0-9]+\.){3}[0-9]+$ ]]
}

pause_if_interactive() {
  if "$interactive"; then
    printf '\n' >&2
    ask '按回车返回主菜单' '' || true
  fi
}

require_initialized() {
  if [[ ! -e $config_path ]]; then
    printf '尚未初始化配置，请先选择 1 初始化。\n' >&2
    pause_if_interactive
    return 1
  fi
}

# ── 版本比较与远程更新检查 ──

normalize_ver() {
  local v=${1-}
  v=${v#v}
  v=${v#sbctl-v}
  v=${v%%[[:space:]]*}
  printf '%s' "$v"
}

version_gt() {
  local cand cur top
  cand=$(normalize_ver "$1")
  cur=$(normalize_ver "$2")
  [[ $cand =~ ^[0-9]+(\.[0-9]+)*([-.][0-9A-Za-z.]+)?$ ]] || return 1
  [[ $cur =~ ^[0-9]+(\.[0-9]+)*([-.][0-9A-Za-z.]+)?$ ]] || return 1
  [[ $cand != "$cur" ]] || return 1
  top=$(printf '%s\n%s\n' "$cand" "$cur" | sort -V 2>/dev/null | tail -n1)
  [[ $top == "$cand" ]]
}

version_cache_path() {
  local dir=$prefix/var/lib/vps-sub-meter
  if [[ $root == / && -d $dir && -w $dir ]]; then
    printf '%s/.ly-version-cache' "$dir"
  else
    printf '%s/.vsm-ly-version-cache' "${XDG_CACHE_HOME:-${HOME:-/tmp}}"
  fi
}

cache_file_mtime() {
  local file=$1
  [[ -r $file ]] || return 1
  stat -c %Y "$file" 2>/dev/null || stat -f %m "$file" 2>/dev/null || return 1
}

read_version_cache() {
  local file k v
  file=$(version_cache_path)
  [[ -r $file ]] || return 1
  CACHE_MTIME_SEEN=$(cache_file_mtime "$file" 2>/dev/null || printf '')
  local ts=0 vsm_v='' xray_v='' sb_v='' ip4=''
  while IFS='=' read -r k v; do
    case $k in
      ts) [[ $v =~ ^[0-9]+$ ]] && ts=$v ;;
      vsm) [[ $v =~ ^[0-9A-Za-z._-]*$ ]] && vsm_v=$v ;;
      xray) [[ $v =~ ^[0-9A-Za-z._-]*$ ]] && xray_v=$v ;;
      singbox) [[ $v =~ ^[0-9A-Za-z._-]*$ ]] && sb_v=$v ;;
      ipv4) [[ $v =~ ^[0-9.]*$ ]] && ip4=$v ;;
    esac
  done <"$file"
  LATEST_VSM_VER=$vsm_v
  LATEST_XRAY_VER=$xray_v
  LATEST_SB_VER=$sb_v
  [[ -n $ip4 ]] && VPS_PUB_IPV4_CACHE=$ip4
  local now
  now=$(date +%s 2>/dev/null || printf '0')
  ((ts > 0 && now - ts < VERSION_CACHE_TTL))
}

invalidate_version_cache() {
  local file
  file=$(version_cache_path)
  rm -f "$file" 2>/dev/null || true
  CACHE_MTIME_SEEN=''
}

fetch_github_latest_tag() {
  local repo=$1 eff tag=''
  eff=$(curl -fsSLI --disable --noproxy '*' --connect-timeout 2 --max-time 3 \
    -o /dev/null -w '%{url_effective}' "https://github.com/${repo}/releases/latest" 2>/dev/null || true)
  if [[ $eff == */releases/tag/* ]]; then
    tag=${eff##*/releases/tag/}
    tag=${tag%%\?*}
    tag=${tag%%#*}
  fi
  if [[ -z $tag ]]; then
    tag=$(curl -fsSL --disable --noproxy '*' --connect-timeout 2 --max-time 3 \
      -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/${repo}/releases/latest" 2>/dev/null \
      | jq -r '.tag_name // empty' 2>/dev/null || true)
  fi
  tag=$(normalize_ver "$tag")
  [[ $tag =~ ^[0-9]+(\.[0-9]+)+([-.][0-9A-Za-z.]+)?$ ]] && printf '%s' "$tag"
}

refresh_version_cache_worker() {
  local file dir tmp vsm_v='' xray_v='' sb_v='' ip4='' now work
  file=$(version_cache_path)
  dir=$(dirname "$file")
  mkdir -p "$dir" 2>/dev/null || return 0
  work=$(mktemp -d 2>/dev/null || return 0)
  (fetch_github_latest_tag 'xiaolingxiaoying/vps-sub-meter' >"$work/vsm" 2>/dev/null || true) &
  (fetch_github_latest_tag 'XTLS/Xray-core' >"$work/xray" 2>/dev/null || true) &
  (fetch_github_latest_tag 'SagerNet/sing-box' >"$work/sb" 2>/dev/null || true) &
  (
    local ip=''
    ip=$(curl -4 --disable --noproxy '*' --fail --silent --connect-timeout 2 --max-time 3 \
      https://api.ipify.org 2>/dev/null || true)
    [[ $ip =~ ^([0-9]+\.){3}[0-9]+$ ]] && printf '%s' "$ip" >"$work/ip4"
  ) &
  wait
  vsm_v=$(cat "$work/vsm" 2>/dev/null || true)
  xray_v=$(cat "$work/xray" 2>/dev/null || true)
  sb_v=$(cat "$work/sb" 2>/dev/null || true)
  ip4=$(cat "$work/ip4" 2>/dev/null || true)
  rm -rf "$work" 2>/dev/null || true
  now=$(date +%s 2>/dev/null || printf '0')
  (
    umask 077
    tmp=$(mktemp "${dir}/.vsm-ver.XXXXXX") || exit 0
    printf 'ts=%s\nvsm=%s\nxray=%s\nsingbox=%s\nipv4=%s\n' \
      "$now" "$vsm_v" "$xray_v" "$sb_v" "$ip4" >"$tmp"
    mv -f "$tmp" "$file"
  ) 2>/dev/null || true
}

start_version_check_async() {
  [[ $root == / ]] || return 0
  "$interactive" || return 0
  if [[ -n ${version_bg_pid:-} ]] && kill -0 "$version_bg_pid" 2>/dev/null; then
    return 0
  fi
  refresh_version_cache_worker </dev/null >/dev/null 2>&1 &
  version_bg_pid=$!
}

evaluate_update_flags() {
  HAS_VSM_UPDATE=0
  HAS_XRAY_UPDATE=0
  HAS_SB_UPDATE=0
  if [[ -n $LATEST_VSM_VER && -n $CUR_VSM_VER ]] && version_gt "$LATEST_VSM_VER" "$CUR_VSM_VER"; then
    HAS_VSM_UPDATE=1
  fi
  if [[ -n $LATEST_XRAY_VER && $CUR_XRAY_VER != '未安装' ]] && version_gt "$LATEST_XRAY_VER" "$CUR_XRAY_VER"; then
    HAS_XRAY_UPDATE=1
  fi
  if [[ -n $LATEST_SB_VER && $CUR_SB_VER != '未安装' ]] && version_gt "$LATEST_SB_VER" "$CUR_SB_VER"; then
    HAS_SB_UPDATE=1
  fi
}

# ── VPS 状态、内核状态、节点/证书与 SSH 命令采集 ──

is_private_ipv4() {
  local ip=$1
  [[ $ip =~ ^(10\.|127\.|169\.254\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.) ]]
}

collect_vps_info() {
  if [[ $root != / ]]; then
    VPS_OS='沙箱环境'
    VPS_KERNEL='—'
    VPS_CPU='—'
    VPS_VIRT='沙箱'
    VPS_BBR='—'
    VPS_IPV4='无'
    VPS_IPV6='无'
    VPS_INFO_LOADED=1
    return 0
  fi

  if ((VPS_INFO_LOADED == 0)); then
    if [[ -r /etc/os-release ]]; then
      VPS_OS=$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-${NAME:-Linux}}")
    else
      VPS_OS=$(uname -s 2>/dev/null || printf 'Linux')
    fi
    VPS_KERNEL=$(uname -r 2>/dev/null || printf '未知')

    local cores cpu_model
    cores=$(nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null || printf '1')
    cpu_model=$(awk -F: '
      /^[[:space:]]*(model name|Hardware|Processor|cpu model)[[:space:]]*:/ {
        v=$2
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
        gsub(/[[:space:]]+/, " ", v)
        if (v != "") { print v; exit }
      }' /proc/cpuinfo 2>/dev/null || true)
    if [[ -n $cpu_model ]]; then
      ((${#cpu_model} > 28)) && cpu_model="${cpu_model:0:25}..."
      VPS_CPU="${cores}核 · ${cpu_model}"
    else
      VPS_CPU="${cores}核"
    fi

    local virt=''
    if command -v systemd-detect-virt >/dev/null 2>&1; then
      virt=$(systemd-detect-virt 2>/dev/null || true)
    fi
    case ${virt,,} in
      none) VPS_VIRT='物理机 / 无' ;;
      kvm) VPS_VIRT='KVM' ;;
      qemu) VPS_VIRT='QEMU' ;;
      vmware) VPS_VIRT='VMware' ;;
      microsoft|hyperv) VPS_VIRT='Hyper-V' ;;
      xen) VPS_VIRT='Xen' ;;
      lxc) VPS_VIRT='LXC' ;;
      openvz) VPS_VIRT='OpenVZ' ;;
      docker) VPS_VIRT='Docker' ;;
      wsl) VPS_VIRT='WSL' ;;
      '') VPS_VIRT='未知' ;;
      *) VPS_VIRT=$virt ;;
    esac
    VPS_INFO_LOADED=1
  fi

  local cc qdisc
  cc=$(tr -d '[:space:]' </proc/sys/net/ipv4/tcp_congestion_control 2>/dev/null || true)
  qdisc=$(tr -d '[:space:]' </proc/sys/net/core/default_qdisc 2>/dev/null || true)
  if [[ -n $cc ]]; then
    VPS_BBR=${qdisc:+"$cc ($qdisc)"}
    VPS_BBR=${VPS_BBR:-$cc}
  else
    VPS_BBR='未知'
  fi

  local ip4='' addr cfg_ip=''
  if command -v ip >/dev/null 2>&1; then
    while read -r addr; do
      addr=${addr%%/*}
      [[ -n $addr ]] || continue
      if ! is_private_ipv4 "$addr"; then
        ip4=$addr
        break
      fi
    done < <(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}')
  fi
  if [[ -z $ip4 && -n $DASH_CONFIG_JSON ]]; then
    cfg_ip=$(jq -r '.public_ip // empty' <<<"$DASH_CONFIG_JSON" 2>/dev/null || true)
    [[ $cfg_ip =~ ^([0-9]+\.){3}[0-9]+$ ]] && ! is_private_ipv4 "$cfg_ip" && ip4=$cfg_ip
  fi
  if [[ -z $ip4 && -n $VPS_PUB_IPV4_CACHE ]]; then
    ip4=$VPS_PUB_IPV4_CACHE
  fi
  VPS_IPV4=${ip4:-无}

  local ip6=''
  if command -v ip >/dev/null 2>&1; then
    while read -r addr; do
      addr=${addr%%/*}
      [[ -n $addr ]] || continue
      case ${addr,,} in
        fe80:*|fc*|fd*) continue ;;
        *) ip6=$addr; break ;;
      esac
    done < <(ip -6 -o addr show scope global 2>/dev/null | awk '{print $4}')
  fi
  VPS_IPV6=${ip6:-无}
}

svc_active_label() {
  local unit=$1 state='未知'
  if [[ $root != / ]] || ! command -v systemctl >/dev/null 2>&1; then
    printf '—'
    return 0
  fi
  state=$(systemctl is-active "$unit" 2>/dev/null || true)
  case $state in
    active) paint 32 '运行中' ;;
    activating|reloading) paint 33 '启动中' ;;
    failed) paint 31 '异常' ;;
    inactive|dead|'') paint 2 '未运行' ;;
    *) printf '%s' "$state" ;;
  esac
}

collect_core_and_versions() {
  local force_ver=${1:-0}
  local vsm_ver='' xray_ver='' sb_ver=''
  if [[ -n $DASH_STATUS_JSON ]]; then
    vsm_ver=$(jq -r '.version // empty' <<<"$DASH_STATUS_JSON" 2>/dev/null || true)
  fi
  if [[ -z $vsm_ver && -x $prefix/usr/local/bin/vps-sub-meter ]]; then
    vsm_ver=$(vsm --version 2>/dev/null | awk '{print $NF; exit}' || true)
  fi
  CUR_VSM_VER=$(normalize_ver "$vsm_ver")

  if [[ $root == / && -x /usr/local/bin/xray ]]; then
    xray_ver=$(/usr/local/bin/xray version 2>/dev/null | awk 'NR==1{print $2}' || true)
    CUR_XRAY_VER=$(normalize_ver "${xray_ver:-未知}")
  elif [[ -x $prefix/usr/local/bin/xray ]]; then
    CUR_XRAY_VER='已部署'
  else
    CUR_XRAY_VER='未安装'
  fi

  if [[ $root == / && -x /usr/local/bin/sing-box ]]; then
    sb_ver=$(/usr/local/bin/sing-box version 2>/dev/null | awk 'NR==1{print $3}' || true)
    CUR_SB_VER=$(normalize_ver "${sb_ver:-未知}")
  elif [[ -x $prefix/usr/local/bin/sing-box ]]; then
    CUR_SB_VER='已部署'
  else
    CUR_SB_VER='未安装'
  fi

  SVC_VSM_STATE=$(svc_active_label 'vps-sub-meter.service')
  SVC_XRAY_STATE=$(svc_active_label 'vps-sub-meter-xray.service')
  SVC_SB_STATE=$(svc_active_label 'vps-sub-meter-singbox.service')
  SVC_ARGO_STATE=$(svc_active_label 'vps-sub-meter-argo.service')

  if [[ $root == / ]] && "$interactive"; then
    local cache_file
    cache_file=$(version_cache_path)
    if ((force_ver)) || [[ ! -r $cache_file ]]; then
      refresh_version_cache_worker
      read_version_cache >/dev/null 2>&1 || true
    elif ! read_version_cache; then
      start_version_check_async
    fi
  fi
  evaluate_update_flags
}

detect_cert_summary() {
  local cfg=$1 domain sni cert subject issuer end_line end_str end_ts now_ts days
  local kind='' days_txt='' auto_suffix=''
  local le_pem acme_pem=''
  DASH_CERT_AUTO_STATE='未知'
  DASH_CERT_RENEW_FAILED=0
  DASH_CERT_RENEW_ERR=''
  DASH_CERT_RENEW_TIME=''
  DASH_CERT_RENEW_COUNT=0

  domain=$(jq -r '.domain | gsub("^\\s+|\\s+$"; "")' <<<"$cfg" 2>/dev/null || true)
  if is_ip_host "$domain"; then
    sni='www.bing.com'
  else
    sni=$domain
  fi
  cert="$prefix/var/lib/vps-sub-meter/certificates/$sni/fullchain.pem"
  if [[ ! -s $cert ]]; then
    DASH_CERT_SUMMARY="$(paint 33 "未签发 ($sni)")"
    return 0
  fi
  subject=$(openssl x509 -in "$cert" -noout -subject 2>/dev/null | sed 's/^subject=[[:space:]]*//' || true)
  issuer=$(openssl x509 -in "$cert" -noout -issuer 2>/dev/null | sed 's/^issuer=[[:space:]]*//' || true)
  end_line=$(openssl x509 -in "$cert" -noout -enddate 2>/dev/null || true)
  end_str=${end_line#notAfter=}

  le_pem="${prefix}/etc/letsencrypt/live/${sni}/fullchain.pem"
  if [[ -s "${prefix}/root/.acme.sh/${sni}_ecc/fullchain.cer" ]]; then
    acme_pem="${prefix}/root/.acme.sh/${sni}_ecc/fullchain.cer"
  elif [[ -s "${prefix}/root/.acme.sh/${sni}/fullchain.cer" ]]; then
    acme_pem="${prefix}/root/.acme.sh/${sni}/fullchain.cer"
  fi

  if is_ip_host "$domain" || [[ -n $subject && $subject == "$issuer" ]]; then
    kind='自签证书'
  elif [[ $issuer == *"Let's Encrypt"* ]]; then
    if [[ -s $le_pem ]] && cmp -s "$cert" "$le_pem" && [[ -z $acme_pem || ! $(cmp -s "$cert" "$acme_pem"; echo $?) -eq 0 ]]; then
      kind="Certbot / Let's Encrypt"
    elif [[ -n $acme_pem ]] && cmp -s "$cert" "$acme_pem"; then
      kind="ACME / Let's Encrypt"
    elif [[ -d "${prefix}/root/.acme.sh/${sni}_ecc" || -d "${prefix}/root/.acme.sh/${sni}" ]]; then
      kind="ACME / Let's Encrypt"
    elif [[ -d "${prefix}/etc/letsencrypt/live/${sni}" ]]; then
      kind="Certbot / Let's Encrypt"
    else
      kind="Let's Encrypt"
    fi
  else
    local ca_name
    ca_name=$(sed -n 's/.*O[[:space:]]*=[[:space:]]*\([^,/]*\).*/\1/p' <<<"$issuer" | head -n1)
    if [[ -z $ca_name ]]; then
      ca_name=$(sed -n 's/.*CN[[:space:]]*=[[:space:]]*\([^,/]*\).*/\1/p' <<<"$issuer" | head -n1)
    fi
    kind="CA 证书${ca_name:+ ($ca_name)}"
  fi

  if [[ -n $end_str ]]; then
    end_ts=$(date -d "$end_str" +%s 2>/dev/null || printf '')
    now_ts=$(date +%s 2>/dev/null || printf '')
    if [[ $end_ts =~ ^[0-9]+$ && $now_ts =~ ^[0-9]+$ ]]; then
      days=$(( (end_ts - now_ts) / 86400 ))
      if ((days < 0)); then
        days_txt=$(paint 31 '已过期')
      elif ((days <= 30)); then
        days_txt=$(paint 33 "剩 ${days} 天")
      else
        days_txt=$(paint 32 "有效 · 剩 ${days} 天")
      fi
    fi
  fi
  if [[ -z $days_txt ]]; then
    if openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1; then
      days_txt=$(paint 32 '有效')
    else
      days_txt=$(paint 31 '已过期')
    fi
  fi

  if ! is_ip_host "$domain"; then
    if [[ $root == / ]] && command -v systemctl >/dev/null 2>&1; then
      DASH_CERT_AUTO_STATE=$(systemctl is-active vps-sub-meter-cert-renew.timer 2>/dev/null || printf 'inactive')
      if [[ $DASH_CERT_AUTO_STATE == active ]]; then
        auto_suffix=" · $(paint 32 '自动续期')"
      fi
    fi
    if [[ -n $DASH_STATUS_JSON ]]; then
      local renew_status
      renew_status=$(jq -r '.cert_renewal.status // empty' <<<"$DASH_STATUS_JSON" 2>/dev/null || true)
      if [[ $renew_status == failed ]]; then
        local r_ts r_tz r_cnt r_err
        DASH_CERT_RENEW_FAILED=1
        r_ts=$(jq -r '.cert_renewal.attempted_at // 0' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf '0')
        r_tz=$(jq -r '.traffic.display_tz // "UTC"' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf 'UTC')
        r_cnt=$(jq -r '.cert_renewal.consecutive_failures // 1' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf '1')
        r_err=$(jq -r '.cert_renewal.error // "未知错误"' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf '未知错误')
        if [[ $r_ts =~ ^[0-9]+$ ]] && ((r_ts > 0)); then
          DASH_CERT_RENEW_TIME=$(TZ="$r_tz" date -d "@${r_ts}" '+%Y-%m-%d %H:%M' 2>/dev/null || printf '%s' "$r_ts")
        else
          DASH_CERT_RENEW_TIME='未知时间'
        fi
        DASH_CERT_RENEW_COUNT=$r_cnt
        DASH_CERT_RENEW_ERR=$r_err
        auto_suffix=" · $(paint '1;31' '续期失败')"
      fi
    fi
  fi
  DASH_CERT_SUMMARY="${kind} (${sni} · ${days_txt})${auto_suffix}"
}

format_node_chip() {
  local cfg=$1 key=$2 label=$3 enabled port hop extra=''
  enabled=$(jq -r --arg k "$key" '.nodes[$k].enabled' <<<"$cfg" 2>/dev/null || printf 'false')
  port=$(jq -r --arg k "$key" '.nodes[$k].port' <<<"$cfg" 2>/dev/null || printf '?')
  if [[ $key == hysteria2_hop ]]; then
    hop=$(jq -r '.nodes.hysteria2_hop.hop_range // empty' <<<"$cfg" 2>/dev/null || true)
    [[ -n $hop ]] && extra=" 跳${hop}"
  fi
  if [[ $enabled == true ]]; then
    printf '%s:%s%s' "$label" "$(paint 36 "$port")" "${extra:+$(paint 2 "($extra)")}"
  else
    printf '%s' "$(paint 2 "${label}:${port}(停用)")"
  fi
}

collect_node_and_cert_info() {
  DASH_CONFIG_JSON=''
  DASH_CERT_SUMMARY='未初始化'
  DASH_REALITY_SNI='—'
  DASH_NODES_LINE1=''
  DASH_NODES_LINE2=''

  ((DASH_INITIALIZED)) || return 0
  DASH_CONFIG_JSON=$(vsm config show 2>/dev/null || true)
  [[ -n $DASH_CONFIG_JSON ]] || return 0

  detect_cert_summary "$DASH_CONFIG_JSON"

  local decoy_sni reality_on
  decoy_sni=$(jq -r '.nodes.vless_reality_pqc.decoy_sni // empty' <<<"$DASH_CONFIG_JSON" 2>/dev/null || true)
  reality_on=$(jq -r '.nodes.vless_reality_pqc.enabled' <<<"$DASH_CONFIG_JSON" 2>/dev/null || printf 'true')
  if [[ -n $decoy_sni ]]; then
    if [[ $reality_on == true ]]; then
      DASH_REALITY_SNI=$decoy_sni
    else
      DASH_REALITY_SNI="${decoy_sni} $(paint 2 '(节点已停用)')"
    fi
  else
    DASH_REALITY_SNI='未设置'
  fi

  local n1 n2 n3 n4 n5
  n1=$(format_node_chip "$DASH_CONFIG_JSON" vless_reality_pqc 'REALITY')
  n2=$(format_node_chip "$DASH_CONFIG_JSON" vless_xhttp_pqc 'XHTTP')
  n3=$(format_node_chip "$DASH_CONFIG_JSON" anytls_tcp 'AnyTLS')
  n4=$(format_node_chip "$DASH_CONFIG_JSON" hysteria2_hop 'Hy2')
  n5=$(format_node_chip "$DASH_CONFIG_JSON" tuic_bbr 'TUIC')
  DASH_NODES_LINE1="${n1}  ${n2}  ${n3}"
  DASH_NODES_LINE2="${n4}  ${n5}"
}

build_panel_ssh_command() {
  local ssh_user host ssh_port='' port_opt=''
  ssh_user=${SUDO_USER:-${USER:-$(id -un 2>/dev/null || printf 'root')}}
  host=''
  if [[ -n $DASH_CONFIG_JSON ]]; then
    host=$(jq -r '.domain | gsub("^\\s+|\\s+$"; "")' <<<"$DASH_CONFIG_JSON" 2>/dev/null || true)
    if [[ -z $host ]]; then
      host=$(jq -r '.public_ip | gsub("^\\s+|\\s+$"; "")' <<<"$DASH_CONFIG_JSON" 2>/dev/null || true)
    fi
  fi
  if [[ -z $host ]]; then
    if [[ $VPS_IPV4 != '无' ]]; then
      host=$VPS_IPV4
    elif [[ $VPS_IPV6 != '无' ]]; then
      host="[$VPS_IPV6]"
    else
      host='<VPS_IP>'
    fi
  elif [[ $host == *:* && $host != \[*\] ]]; then
    host="[$host]"
  fi
  if [[ -n ${SSH_CONNECTION:-} ]]; then
    ssh_port=$(awk '{print $4}' <<<"$SSH_CONNECTION" 2>/dev/null || true)
  fi
  if [[ $ssh_port =~ ^[0-9]+$ && $ssh_port != 22 ]]; then
    port_opt="-p ${ssh_port} "
  fi
  DASH_PANEL_SSH_CMD="ssh -t -o ExitOnForwardFailure=yes ${port_opt}-L 127.0.0.1:8787:127.0.0.1:8787 ${ssh_user}@${host} 'sudo /usr/local/bin/vps-sub-meter panel'"
}

collect_dashboard_data() {
  local force_ver=${1:-0}
  if [[ -e $config_path ]]; then
    DASH_INITIALIZED=1
    DASH_STATUS_JSON=$(vsm status --json 2>/dev/null || true)
  else
    DASH_INITIALIZED=0
    DASH_STATUS_JSON=''
  fi
  collect_node_and_cert_info
  collect_vps_info
  if ((force_ver)); then
    invalidate_version_cache
  fi
  collect_core_and_versions "$force_ver"
  build_panel_ssh_command
}

format_iec_bytes() {
  local bytes=${1:-0}
  awk -v b="$bytes" 'BEGIN {
    if (b >= 1099511627776) printf "%.2f TiB", b/1099511627776;
    else if (b >= 1073741824) printf "%.2f GiB", b/1073741824;
    else if (b >= 1048576) printf "%.2f MiB", b/1048576;
    else if (b >= 1024) printf "%.2f KiB", b/1024;
    else printf "%.0f B", b;
  }'
}

format_ver_badge() {
  local cur=$1 svc=$2 has_upd=$3 latest=$4
  local cur_disp upd_disp=''
  if [[ $cur == '未安装' || $cur == '已部署' || $cur == '未知' ]]; then
    cur_disp=$(paint 33 "$cur")
  else
    cur_disp="v${cur}"
  fi
  if ((has_upd)) && [[ -n $latest ]]; then
    upd_disp=" $(paint '1;33' "→ 新版 v${latest}")"
  elif [[ -n $latest && $cur != '未安装' && $cur != '已部署' ]]; then
    upd_disp=" $(paint 2 '(最新)')"
  fi
  printf '[%s] %s%s' "$svc" "$cur_disp" "$upd_disp"
}

menu_entry_str() {
  local num=$1 label=$2 col=${3:-32} formatted
  printf -v formatted '%2s.' "$num"
  printf '  %s %s' "$(paint "$col" "$formatted")" "$label"
}

section_half_bar() {
  local title=$1 width=$2 tw fill
  tw=$(vis_width "$title")
  fill=$((width - tw - 6))
  ((fill < 2)) && fill=2
  printf ' %s%s%s' "$(paint 2 '── ')" "$(paint '1;36' "$title")" "$(paint 2 " $(repeat_char '─' "$fill")")"
}

render_main_menu_body() {
  local w compact=${1:-0} title_right=''

  if "$interactive"; then
    w=$(panel_width)
    if ((HAS_VSM_UPDATE)); then
      title_right=$(paint '1;33' "★ 发现新版 v${LATEST_VSM_VER}")
    else
      title_right=$(paint 2 "v${CUR_VSM_VER:-?}")
    fi

    hr_line '━' '36'
    local title_left
    title_left="  $(paint '1;36' 'vps-sub-meter · 管理菜单')"
    if ((TERM_COLS >= 72)); then
      local lw rw gap
      lw=$(vis_width "$title_left")
      rw=$(vis_width "$title_right")
      gap=$((w - lw - rw))
      if ((gap >= 2)); then
        printf '%s%*s%s\n' "$title_left" "$gap" '' "$title_right"
      else
        printf '%s  %s\n' "$title_left" "$title_right"
      fi
    else
      printf '%s  %s\n' "$title_left" "$title_right"
    fi
    hr_line '─' '2'

    # ── 1. 节点、证书与订阅 ──
    if ((DASH_INITIALIZED)) && [[ -n $DASH_CONFIG_JSON ]]; then
      local host_addr proto_label https_on
      host_addr=$(jq -r '.address // empty' <<<"$DASH_STATUS_JSON" 2>/dev/null || true)
      https_on=$(jq -r '.https_enabled // false' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf 'false')
      if [[ $https_on == true ]]; then
        proto_label=$(paint 32 "HTTPS (${host_addr})")
      else
        proto_label=$(paint 33 "HTTP 明文 (${host_addr})")
      fi
      print_kv_pair '订阅地址' "$proto_label" '证书形式' "$DASH_CERT_SUMMARY"
      print_kv_pair 'REALITY 伪装' "$(paint 36 "$DASH_REALITY_SNI")" '主力端口' "$DASH_NODES_LINE1"
      print_kv_pair '备用/UDP' "$DASH_NODES_LINE2"

      if [[ -n $DASH_STATUS_JSON ]]; then
        local up_b down_b tot_b lim_b tz reset_ts reset_str iface gen_id node_cnt argo_on pending
        up_b=$(jq -r '.traffic.upload_bytes // 0' <<<"$DASH_STATUS_JSON")
        down_b=$(jq -r '.traffic.download_bytes // 0' <<<"$DASH_STATUS_JSON")
        tot_b=$(jq -r '.traffic.total_used_bytes // 0' <<<"$DASH_STATUS_JSON")
        lim_b=$(jq -r '.traffic.monthly_limit_bytes // 0' <<<"$DASH_STATUS_JSON")
        tz=$(jq -r '.traffic.display_tz // "UTC"' <<<"$DASH_STATUS_JSON")
        reset_ts=$(jq -r '.traffic.next_reset_ts // 0' <<<"$DASH_STATUS_JSON")
        iface=$(jq -r '.interface // "eth0"' <<<"$DASH_STATUS_JSON")
        gen_id=$(jq -r '.generation.id // empty' <<<"$DASH_STATUS_JSON")
        node_cnt=$(jq -r '.generation.direct_nodes | length // 0' <<<"$DASH_STATUS_JSON" 2>/dev/null || printf '0')
        argo_on=$(jq -r '.argo_enabled // false' <<<"$DASH_STATUS_JSON")
        pending=$(jq -r '.pending_changes // false' <<<"$DASH_STATUS_JSON")

        local lim_str traffic_str reset_disp
        lim_str=$( ((lim_b > 0)) && format_iec_bytes "$lim_b" || printf '不限' )
        traffic_str="$(format_iec_bytes "$tot_b") / ${lim_str} (↑$(format_iec_bytes "$up_b") ↓$(format_iec_bytes "$down_b"))"
        reset_disp=$(TZ="$tz" date -d "@${reset_ts}" '+%Y-%m-%d %H:%M' 2>/dev/null || printf '%s' "$reset_ts")
        print_kv_pair "流量 ($iface)" "$traffic_str" '下次重置' "${reset_disp} (${tz})"

        local batch_str argo_str
        batch_str=${gen_id:+"${gen_id} (${node_cnt} 节点)"}
        batch_str=${batch_str:-$(paint 33 '未发布')}
        if [[ $argo_on == true ]]; then
          argo_str="已启用 [$SVC_ARGO_STATE]"
        else
          argo_str=$(paint 2 '未启用')
        fi
        print_kv_pair '当前批次' "$batch_str" 'Argo 隧道' "$argo_str"
        if [[ $pending == true ]]; then
          printf '  %s\n' "$(paint '1;33' '配置待应用: 主配置与已发布批次不一致，请选择「8. 应用配置」生效')"
        fi
        if ((DASH_CERT_RENEW_FAILED)); then
          local err_disp=$DASH_CERT_RENEW_ERR
          ((${#err_disp} > 96)) && err_disp="${err_disp:0:93}..."
          printf '  %s\n' "$(paint '1;31' "证书续期失败 (${DASH_CERT_RENEW_TIME} · 连续 ${DASH_CERT_RENEW_COUNT} 次): ${err_disp}")"
          printf '  %s\n' "$(paint 31 '  请选择「12. 证书管理」查看详情或手动重试续期')"
        fi
      fi
    else
      printf '  %s\n' "$(paint '1;33' '尚未初始化部署，请选择「1. 初始化 / 重新配置」开始部署。')"
    fi

    # ── 2. 内核版本与服务状态 ──
    ((compact == 0)) && hr_line '─' '2'
    local vsm_badge xray_badge sb_badge
    vsm_badge=$(format_ver_badge "${CUR_VSM_VER:-?}" "$SVC_VSM_STATE" "$HAS_VSM_UPDATE" "$LATEST_VSM_VER")
    xray_badge=$(format_ver_badge "$CUR_XRAY_VER" "$SVC_XRAY_STATE" "$HAS_XRAY_UPDATE" "$LATEST_XRAY_VER")
    sb_badge=$(format_ver_badge "$CUR_SB_VER" "$SVC_SB_STATE" "$HAS_SB_UPDATE" "$LATEST_SB_VER")
    print_kv_pair '控制面 vsm' "$vsm_badge" 'Xray 内核' "$xray_badge"
    print_kv_pair 'sing-box 内核' "$sb_badge"

    # ── 3. VPS 状态 ──
    ((compact == 0)) && hr_line '─' '2'
    local bbr_disp ipv4_disp ipv6_disp
    if [[ $VPS_BBR == bbr* ]]; then
      bbr_disp=$(paint 32 "$VPS_BBR")
    else
      bbr_disp=$(paint 33 "$VPS_BBR")
    fi
    ipv4_disp=$( [[ $VPS_IPV4 == '无' ]] && paint 2 '无' || printf '%s' "$VPS_IPV4" )
    ipv6_disp=$( [[ $VPS_IPV6 == '无' ]] && paint 2 '无' || printf '%s' "$VPS_IPV6" )
    print_kv_pair '系统' "$VPS_OS" '内核' "$VPS_KERNEL"
    print_kv_pair '处理器' "$VPS_CPU" '虚拟化' "$VPS_VIRT"
    print_kv_pair 'BBR算法' "$bbr_disp" 'IPv4 地址' "$ipv4_disp"
    print_kv_pair 'IPv6 地址' "$ipv6_disp"
  else
    menu_heading 'vps-sub-meter · 管理菜单'
    if [[ -e $config_path ]]; then
      vsm status
    else
      paint 33 '尚未初始化，请选择 1 开始部署。'
      printf '\n'
    fi
  fi

  if "$interactive" && ((TERM_COLS >= 88)); then
    local col_w=$((w / 2))
    local -a left_rows=(
      "$(section_half_bar '订阅与流量' "$col_w")"
      "$(menu_entry_str 2 '查看订阅与二维码')"
      "$(menu_entry_str 3 '管理订阅凭据')"
      "$(menu_entry_str 10 '流量账期与校准')"
      "$(section_half_bar '系统维护' "$col_w")"
      "$(menu_entry_str 4 '轮换节点密钥')"
      "$(menu_entry_str 5 '更新内核 (Xray / sing-box)')"
      "$(menu_entry_str 13 '查看运行日志')"
      "$(menu_entry_str 9 '卸载 vps-sub-meter' 31)"
      "$(menu_entry_str 0 '退出')"
    )
    local -a right_rows=(
      "$(section_half_bar '部署与配置' "$col_w")"
      "$(menu_entry_str 1 '初始化 / 重新配置')"
      "$(menu_entry_str 6 '配置 / 切换域名 (HTTPS)')"
      "$(menu_entry_str 7 '管理 Argo 隧道')"
      "$(menu_entry_str 8 '应用配置')"
      "$(menu_entry_str 11 '节点端口设置')"
      "$(menu_entry_str 12 '证书管理')"
      ""
      ""
      ""
    )
    ((compact == 0)) && printf '\n'
    local idx
    for ((idx = 0; idx < ${#left_rows[@]}; idx++)); do
      printf '%s%s\n' "$(pad_right "${left_rows[idx]}" "$col_w")" "${right_rows[idx]}"
    done
  else
    menu_section '订阅与流量'
    menu_item 2 '查看订阅与二维码'
    menu_item 3 '管理订阅凭据'
    menu_item 10 '流量账期与校准'

    menu_section '部署与配置'
    menu_item 1 '初始化 / 重新配置'
    menu_item 6 '配置 / 切换域名 (HTTPS)'
    menu_item 7 '管理 Argo 隧道'
    menu_item 8 '应用配置'
    menu_item 11 '节点端口设置'
    menu_item 12 '证书管理'

    menu_section '系统维护'
    menu_item 4 '轮换节点密钥'
    menu_item 5 '更新内核 (Xray / sing-box)'
    menu_item 13 '查看运行日志'
    menu_item 9 '卸载 vps-sub-meter' 31
    ((compact == 0)) && printf '\n'
    menu_item 0 '退出'
  fi
  if "$interactive"; then
    hr_line '─' '2'
    printf '  %s\n' "$(paint 2 '网页管理面板 (在本机运行 SSH 转发后打开 http://127.0.0.1:8787/):')"
    printf '  %s\n' "$(paint 36 "$DASH_PANEL_SSH_CMD")"
  fi
}

render_main_menu() {
  local compact=0 frame=''
  refresh_term_size
  LAST_RAW_COLS=$RAW_TERM_COLS
  LAST_RAW_LINES=$RAW_TERM_LINES
  ((TERM_LINES < 30)) && compact=1

  if "$interactive"; then
    frame=$(render_main_menu_body "$compact")
    printf '\033[H\033[2J\033[3J%s\n' "$frame"
  else
    render_main_menu_body "$compact"
  fi
}

# 返回码: 0=已选中编号($answer), 1=退出(0或EOF), 2=窗口尺寸真实变化需重绘, 3=手动刷新(r)
read_main_choice() {
  local rc settle prev_r prev_c
  while true; do
    paint '1;33' '请输入编号（0–13，r 刷新）' >&2
    printf ': ' >&2
    while true; do
      winch_resized=0
      IFS= read -r -t 0.5 answer
      rc=$?
      if ((rc == 0)); then
        answer=${answer#"${answer%%[![:space:]]*}"}
        answer=${answer%"${answer##*[![:space:]]}"}
        if [[ $answer == 0 ]]; then
          return 1
        elif [[ $answer == r || $answer == R ]]; then
          return 3
        elif [[ $answer =~ ^[1-9][0-9]?$ ]] && ((answer <= 13)); then
          return 0
        fi
        printf '\n请输入 0–13 之间的编号（或输入 r 刷新）。\n' >&2
        break
      elif ((rc == 1)); then
        return 1
      else
        if query_raw_term_size && ((RAW_TERM_LINES != LAST_RAW_LINES || RAW_TERM_COLS != LAST_RAW_COLS)); then
          settle=0
          prev_r=$RAW_TERM_LINES
          prev_c=$RAW_TERM_COLS
          while ((settle < 3)); do
            sleep 0.12
            if query_raw_term_size && ((RAW_TERM_LINES == prev_r && RAW_TERM_COLS == prev_c)); then
              break
            fi
            prev_r=$RAW_TERM_LINES
            prev_c=$RAW_TERM_COLS
            ((settle += 1))
          done
          winch_resized=0
          return 2
        fi
        winch_resized=0
      fi
    done
  done
}

# ── 原有子菜单与操作函数 ──

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

direct_nodes=(vless_reality_pqc vless_xhttp_pqc anytls_tcp hysteria2_hop tuic_bbr)

initialize() {
  local current host domain ip interface quota reset_day node candidate email='' mode sni
  local acme_port https_port
  local detected default_ip replace=() show=() port_args=() keep_label='使用模板默认端口'
  menu_heading '初始化 / 重新配置'
  printf '  回车采用方括号中的默认值，输入 0 返回主菜单。\n\n'
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
      detected=$(ip -4 route show default 2>/dev/null | awk '$1 == "default" {for (i = 2; i < NF; i++) if ($i == "dev" && $(i + 1) != "lo") {print $(i + 1); exit}}') || detected=''
      if [[ -z $detected ]]; then
        detected=$(ip -6 route show default 2>/dev/null | awk '$1 == "default" {for (i = 2; i < NF; i++) if ($i == "dev" && $(i + 1) != "lo") {print $(i + 1); exit}}') || detected=''
      fi
      interface=${detected:-$interface}
    fi
  fi
  host=$(jq -r '.domain' <<<"$current") || return 1
  ask '订阅域名（推荐，启用 HTTPS；填公网 IP 则退回明文 HTTP）' "${host:-$default_ip}" || return 0
  host=$answer
  if [[ -z $host ]]; then
    printf '请输入域名或公网 IP。\n' >&2
    pause_if_interactive
    return 1
  elif is_ip_host "$host"; then
    domain=''
    ip=$host
  else
    domain=$host
    ask '节点使用的公网 IP' "$default_ip" || return 0
    ip=$answer
  fi
  quota=$(jq -r '.traffic.monthly_gb' <<<"$current") || return 1
  ask '每月流量 GiB（不限可输入 0.0，0 返回）' "$quota" || return 0
  quota=$answer
  reset_day=$(jq -r '.traffic.reset_day' <<<"$current") || return 1
  ask '每月重置日 (1-31)' "$reset_day" || return 0
  reset_day=$answer
  menu_section '节点端口方案'
  menu_item 1 "$keep_label"
  menu_item 2 '随机分配端口'
  menu_item 3 '逐个设置端口'
  menu_item 0 '返回主菜单'
  choose 3 '选择端口方案' 1 || return 0
  case $answer in
    1) port_args=(--ports=keep) ;;
    2) port_args=(--ports=random) ;;
    3)
      port_args=(--ports=keep)
      for node in "${direct_nodes[@]}"; do ask_node_port "$current" "$node" port_args || return 0; done
      ;;
  esac
  if ! candidate=$(vsm config prepare --domain="$domain" --public-ip="$ip" --interface="$interface" \
    --monthly-gib="$quota" --reset-day="$reset_day" "${port_args[@]}"); then
    pause_if_interactive
    return 1
  fi
  mode=$(jq -r '.mode' <<<"$candidate") || return 1
  sni=$(jq -r '.tls_sni' <<<"$candidate") || return 1
  if [[ $mode != ip ]]; then
    ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 0
    email=$answer
  fi
  menu_section '部署预览'
  jq -r '"部署模式：\(.mode)  网卡：\(.config.interface)  月额度：\(.config.traffic.monthly_gb) GiB  重置日：\(.config.traffic.reset_day)",
    (.config.nodes[] | "  \(.name): \(.port)")' <<<"$candidate" || return 1
  if [[ $mode == ip ]]; then
    printf '警告：未配置域名，订阅使用明文 HTTP。请通过可信渠道导入带证书指纹的客户端配置，或绑定域名启用 HTTPS。\n' >&2
  else
    printf '订阅协议：HTTPS (https://%s/sub/...)\n' "$sni"
  fi
  confirm '确认保存配置并部署' 1 || return 0

  local rc=0
  if [[ $root == / ]]; then
    "$script_dir/cores.sh" ensure || rc=$?
    if ((rc == 0)); then
      "$script_dir/net.sh" tune-sysctl || printf '内核参数调优未全部生效，继续部署。\n' >&2
      if [[ $mode == ip ]]; then
        "$script_dir/cert.sh" self-signed "$sni" || rc=$?
      else
        acme_port=$(jq -r '.config.server.acme_port' <<<"$candidate") || rc=$?
        https_port=$(jq -r '.config.server.https_port' <<<"$candidate") || rc=$?
        if ((rc == 0)); then
          "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
          "$script_dir/cert.sh" issue "$sni" "$email" || rc=$?
        fi
      fi
    fi
  fi
  if ((rc == 0)); then
    jq -c '.config' <<<"$candidate" | vsm config write "${replace[@]}" || rc=$?
  fi
  if ((rc == 0)); then
    vsm apply || rc=$?
  fi
  pause_if_interactive
  return "$rc"
}

manage_ports() {
  require_initialized || return 1
  local current node description index=1 candidate port_args=(--ports=keep) rc=0
  menu_heading '节点端口设置'
  current=$(vsm config show) || { pause_if_interactive; return 1; }
  for node in "${direct_nodes[@]}"; do
    description=$(jq -r --arg node "$node" \
      '.nodes[$node] | "\(.name)  \(.port)\(if .enabled then "" else "（已停用）" end)"' <<<"$current") || return 1
    menu_item "$index" "$description"
    ((index += 1))
  done
  menu_item 6 '使用模板默认端口'
  menu_item 7 '重新随机分配端口'
  menu_item 0 '返回主菜单'
  choose 7 '选择节点或端口方案' 0 || return 0
  case $answer in
    [1-5]) ask_node_port "$current" "${direct_nodes[answer-1]}" port_args || return 0 ;;
    6) port_args=(--ports=defaults) ;;
    7) port_args=(--ports=random) ;;
  esac
  if ! candidate=$(vsm config prepare "${port_args[@]}"); then
    pause_if_interactive
    return 1
  fi
  menu_section '端口预览'
  jq -r '.config.nodes[] | "  \(.name): \(.port)"' <<<"$candidate" || return 1
  if jq -e --argjson current "$current" '.config.nodes == $current.nodes' >/dev/null <<<"$candidate"; then
    printf '节点端口未修改。\n'
    pause_if_interactive
    return 0
  fi
  confirm '确认应用节点端口并更新订阅' 2 || return 0
  jq -c '.config' <<<"$candidate" | vsm config apply || rc=$?
  pause_if_interactive
  return "$rc"
}

subscriptions() {
  require_initialized || return 1
  local links selection=()
  menu_heading '查看订阅与二维码'
  ask '凭据名称（回车使用默认凭据）' '' || return 0
  if [[ -n $answer ]]; then selection=(--token-name "$answer"); fi
  if ! links=$(vsm sub --json "${selection[@]}"); then
    pause_if_interactive
    return 1
  fi
  jq -r '"\n直连订阅", (.direct | to_entries[] | "  \(.key): \(.value)"),
    (if .argo then "\nArgo 订阅", (.argo | to_entries[] | "  \(.key): \(.value)") else empty end)' <<<"$links" || return 1
  local qr_source=direct sb_label='sing-box'
  if jq -e '.argo != null' >/dev/null <<<"$links"; then
    menu_section '二维码订阅来源'
    menu_item 1 '直连'
    menu_item 2 'Argo 隧道'
    menu_item 0 '返回主菜单'
    choose 2 '选择订阅来源' 0 || return 0
    if [[ $answer == 2 ]]; then
      selection+=(--argo)
      qr_source=argo
    fi
  fi
  if ! jq -e --arg src "$qr_source" '.[$src].sing_box != null' >/dev/null <<<"$links"; then
    sb_label='sing-box（当前未启用）'
  fi
  menu_section '二维码格式'
  menu_item 1 'Clash'
  menu_item 2 "$sb_label"
  menu_item 3 'Shadowrocket'
  menu_item 0 '返回主菜单'
  choose 3 '选择客户端格式' 0 || return 0
  case $answer in
    1) selection+=(--qr) ;;
    2) selection+=(--qr-format sing-box) ;;
    3) selection+=(--qr-format shadowrocket) ;;
  esac
  local rc=0
  vsm sub "${selection[@]}" || rc=$?
  pause_if_interactive
  return "$rc"
}

tokens() {
  require_initialized || return 1
  local list name
  menu_heading '管理订阅凭据'
  if ! list=$(vsm token list --json); then
    pause_if_interactive
    return 1
  fi
  if ! jq -r '.[] | [.name, .masked, (if .default then "默认" else "" end)] | @tsv' <<<"$list"; then
    pause_if_interactive
    return 1
  fi
  printf '\n'
  menu_item 1 '新增订阅凭据'
  menu_item 2 '轮换默认凭据'
  menu_item 3 '吊销指定凭据'
  menu_item 0 '返回主菜单'
  choose 3 '选择凭据操作' || return 0
  local rc=0
  case $answer in
    1)
      ask '设备名称' || return 1
      vsm token add -- "$answer" || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
    2)
      confirm '确认轮换默认凭据，旧订阅链接将失效' 2 || return 1
      vsm token rotate || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
    3)
      ask '要吊销的凭据名称' || return 1
      name=$answer
      confirm '确认吊销该凭据' 2 || return 1
      vsm token revoke -- "$name" || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
  esac
}

uninstall_vsm() {
  local flags=() rc=0
  if [[ ! -f $script_dir/uninstall.sh ]]; then
    printf '找不到卸载脚本: %s\n' "$script_dir/uninstall.sh" >&2
    pause_if_interactive
    return 1
  fi
  menu_heading '卸载 vps-sub-meter'
  menu_item 1 '彻底卸载' 31
  menu_item 2 '卸载并保留配置与流量账本'
  menu_item 0 '返回主菜单'
  choose 2 '选择卸载模式' 0 || return 0
  case $answer in
    1) flags=() ;;
    2) flags=(--keep-config) ;;
  esac
  bash "$script_dir/uninstall.sh" --root "$root" "${flags[@]}" || rc=$?
  case $rc in
    0) exit 0 ;;
    10) return 0 ;;
    *) pause_if_interactive; return "$rc" ;;
  esac
}

update_cores() {
  local rc=0 use_latest=0
  if [[ ! -f $script_dir/cores.sh ]]; then
    printf '找不到内核更新脚本: %s\n' "$script_dir/cores.sh" >&2
    pause_if_interactive
    return 1
  fi
  menu_heading '更新代理内核 (Xray / sing-box)'
  printf '  Xray 当前: %s%s\n' "$CUR_XRAY_VER" "${LATEST_XRAY_VER:+  |  GitHub 最新: $LATEST_XRAY_VER}"
  printf '  sing-box 当前: %s%s\n\n' "$CUR_SB_VER" "${LATEST_SB_VER:+  |  GitHub 最新: $LATEST_SB_VER}"
  if ((HAS_XRAY_UPDATE || HAS_SB_UPDATE)) && [[ -n $LATEST_XRAY_VER || -n $LATEST_SB_VER ]]; then
    menu_item 1 '更新到脚本指定稳定版本'
    menu_item 2 '更新到 GitHub 最新 Release 版本'
    menu_item 0 '返回主菜单'
    choose 2 '选择更新目标' 1 || return 0
    [[ $answer == 2 ]] && use_latest=1
  else
    confirm '确认更新 Xray 与 sing-box 内核到脚本指定版本' 1 || return 0
  fi
  if [[ $root == / ]]; then
    if ((use_latest)); then
      XRAY_VERSION="${LATEST_XRAY_VER:-}" SING_BOX_VERSION="${LATEST_SB_VER:-}" \
        bash "$script_dir/cores.sh" update || rc=$?
    else
      bash "$script_dir/cores.sh" update || rc=$?
    fi
  fi
  if ((rc == 0)) && [[ -e $config_path ]]; then
    vsm apply || rc=$?
  fi
  if ((rc == 0)); then
    invalidate_version_cache
  fi
  pause_if_interactive
  return "$rc"
}

view_logs() {
  local unit='' label='' mode=1
  if [[ $root != / ]]; then
    menu_heading '查看运行日志'
    printf '沙箱模式下无法读取宿主机 systemd 日志。\n' >&2
    pause_if_interactive
    return 0
  fi
  if ! command -v journalctl >/dev/null 2>&1; then
    menu_heading '查看运行日志'
    printf '当前系统未检测到 journalctl，无法查看服务日志。\n' >&2
    pause_if_interactive
    return 1
  fi

  while true; do
    menu_heading '查看运行日志'
    menu_item 1 '订阅服务 (vps-sub-meter)'
    menu_item 2 'Xray 内核 (vps-sub-meter-xray)'
    menu_item 3 'sing-box 内核 (vps-sub-meter-singbox)'
    menu_item 4 'Argo 隧道 (vps-sub-meter-argo)'
    menu_item 5 '证书自动续期 (vps-sub-meter-cert-renew)'
    menu_item 6 '全部核心服务综合日志'
    menu_item 0 '返回主菜单'
    choose 6 '选择要查看日志的服务' 0 || return 0
    case $answer in
      1) unit='vps-sub-meter.service'; label='订阅服务' ;;
      2) unit='vps-sub-meter-xray.service'; label='Xray 内核' ;;
      3) unit='vps-sub-meter-singbox.service'; label='sing-box 内核' ;;
      4) unit='vps-sub-meter-argo.service'; label='Argo 隧道' ;;
      5) unit='vps-sub-meter-cert-renew.service'; label='证书自动续期' ;;
      6) unit='ALL'; label='全部核心服务' ;;
    esac

    menu_heading "日志操作 · ${label}"
    menu_item 1 '查看最近 50 行'
    menu_item 2 '查看最近 200 行'
    menu_item 3 '实时跟踪日志 (Ctrl+C 停止返回)'
    menu_item 4 '仅查看警告与错误 (最近 100 条)'
    menu_item 0 '返回上一级'
    choose 4 '选择查看方式' 1 || continue
    mode=$answer

    local -a units=()
    if [[ $unit == ALL ]]; then
      units=(-u vps-sub-meter.service -u vps-sub-meter-xray.service -u vps-sub-meter-singbox.service -u vps-sub-meter-argo.service -u vps-sub-meter-cert-renew.service)
    else
      units=(-u "$unit")
    fi

    clear_screen
    case $mode in
      1)
        journalctl "${units[@]}" -n 50 --no-pager || true
        pause_if_interactive
        ;;
      2)
        if "$interactive" && command -v less >/dev/null 2>&1; then
          journalctl "${units[@]}" -n 200 --no-pager | less -R +G || true
        else
          journalctl "${units[@]}" -n 200 --no-pager || true
          pause_if_interactive
        fi
        ;;
      3)
        printf '正在实时跟踪 %s 日志，按 Ctrl+C 停止并返回...\n\n' "$label" >&2
        trap ':' INT
        journalctl "${units[@]}" -n 30 -f || true
        if "$interactive"; then
          trap 'cleanup; exit 130' INT
        else
          trap - INT
        fi
        printf '\n' >&2
        pause_if_interactive
        ;;
      4)
        journalctl "${units[@]}" -p warning -n 100 --no-pager || true
        pause_if_interactive
        ;;
    esac
  done
}

manage_argo() {
  require_initialized || return 1
  local current enabled td_cur sd_cur default_sd token='' reuse_token=0 has_saved_token=0 td sd updated rc=0 confirm_label='确认更新 Argo 优选 IP'
  local token_file=$prefix/etc/cloudflared/vps-sub-meter-argo.token
  menu_heading '管理 Argo 隧道'
  current=$(vsm config show) || { pause_if_interactive; return 1; }
  enabled=$(jq -r '.argo.enabled' <<<"$current") || return 1
  td_cur=$(jq -r '.argo.tunnel_domain' <<<"$current") || return 1
  sd_cur=$(jq -r '.argo.sub_domain' <<<"$current") || return 1
  if [[ -s $token_file ]] && [[ -n $(tr -d '[:space:]' <"$token_file") ]]; then has_saved_token=1; fi
  printf '当前 Argo 状态: %s' "$([[ $enabled == true ]] && printf '已启用' || printf '未启用')" >&2
  if [[ -n $td_cur ]]; then
    printf ' (tunnel=%s, sub=%s)' "$td_cur" "${sd_cur:-$td_cur}" >&2
  fi
  if ((has_saved_token)); then
    printf ' [已保存 Token]' >&2
  fi
  printf '\n' >&2
  jq -r '.argo | "当前 Argo 优选 IP：" +
    (if .preferred_ips == null then "从文件读取（\(.preferred_ips_file)）"
     elif .preferred_ips == [] then "仅使用隧道域名"
     else (.preferred_ips | join("、")) end)' <<<"$current" >&2 || return 1

  printf '\n'
  menu_item 1 '配置并启用'
  menu_item 2 '停用'
  menu_item 3 '检查本机与公网订阅'
  menu_item 4 '更新回源规则'
  menu_item 5 '更新优选 IP'
  menu_item 0 '返回主菜单'
  choose 5 '选择 Argo 操作' 0 || return 0
  case $answer in
    1|5)
      updated=$current
      if [[ $answer == 1 ]]; then
        if ((has_saved_token)); then
          ask_tunnel_token 'Cloudflare Tunnel Token（回车沿用已保存 Token，0 返回）' || return 0
        else
          ask_tunnel_token || return 0
        fi
        token=$answer
        if [[ -z $token ]]; then
          if ((has_saved_token)); then
            reuse_token=1
          else
            printf 'Tunnel Token 不能为空。\n' >&2
            pause_if_interactive
            return 1
          fi
        fi
        ask 'Argo 节点接入域名 (WebSocket)' "$td_cur" || return 0
        td=$answer
        if [[ -z $td ]]; then
          printf 'tunnel_domain 不能为空。\n' >&2
          pause_if_interactive
          return 1
        fi
        default_sd=$sd_cur
        if [[ -z $default_sd || $default_sd == "$td_cur" ]]; then default_sd=$td; fi
        ask 'Argo 订阅域名 (可与节点域名相同，需指向本隧道)' "$default_sd" || return 0
        sd=${answer:-$td}
        updated=$(jq -c --arg td "$td" --arg sd "$sd" \
          '.argo.enabled = true | .argo.tunnel_domain = $td | .argo.sub_domain = $sd' <<<"$current") || return 1
        confirm_label='确认写入 Argo 配置并生效'
      fi
      printf '%s\n' '最多 32 个公网 IPv4，用逗号或空格分隔。' \
        '回车保留，输入「清空」仅用隧道域名，输入「文件」从文件读取。' >&2
      ask 'Argo 优选 IP（0 返回）' || return 0
      [[ -n $answer || -n $token || $reuse_token -eq 1 ]] || return 0
      if ! updated=$(jq -c --arg input "$answer" '
        if $input == "" then .
        elif $input == "清空" then .argo.preferred_ips = []
        elif $input == "文件" then del(.argo.preferred_ips)
        else
          ($input | gsub("[,，[:space:]]+"; " ") | split(" ") | map(select(length > 0))) as $ips |
          if $ips == [] then error("请输入至少一个 Argo 优选 IP")
          else .argo.preferred_ips = $ips end
        end' <<<"$updated"); then
        pause_if_interactive
        return 1
      fi
      confirm "$confirm_label" 1 || return 0
      vsm config apply <<<"$updated" || rc=$?
      if ((rc == 0)) && [[ $root == / && -f $script_dir/argo.sh ]]; then
        if [[ -n $token ]]; then
          bash "$script_dir/argo.sh" setup --token-stdin <<<"$token" || rc=$?
        elif ((reuse_token)); then
          bash "$script_dir/argo.sh" setup --reuse-token || rc=$?
        fi
      fi
      pause_if_interactive
      return "$rc"
      ;;
    2)
      confirm '确认停用 Argo 隧道并重新应用配置' 2 || return 0
      updated=$(jq -c '.argo.enabled = false' <<<"$current") || return 1
      vsm config apply <<<"$updated" || rc=$?
      if ((rc == 0)) && [[ $root == / && -f $script_dir/argo.sh ]]; then
        bash "$script_dir/argo.sh" disable || true
      fi
      pause_if_interactive
      return "$rc"
      ;;
    3)
      if [[ $enabled != true ]]; then
        printf 'Argo 隧道尚未启用。\n' >&2
        pause_if_interactive
        return 1
      fi
      if [[ $root == / ]]; then
        bash "$script_dir/argo.sh" check || true
        pause_if_interactive
      fi
      ;;
    4)
      if [[ $enabled != true ]]; then
        printf 'Argo 隧道尚未启用。\n' >&2
        pause_if_interactive
        return 1
      fi
      confirm '确认重新生成 Cloudflared 回源规则' 2 || return 0
      if [[ $root == / ]]; then
        bash "$script_dir/argo.sh" refresh --require-token || rc=$?
        pause_if_interactive
        return "$rc"
      fi
      ;;
  esac
}

manage_domain() {
  require_initialized || return 1
  local current cur_domain acme_port https_port new_domain email='' candidate mode sni rc=0
  menu_heading '域名与 HTTPS'
  current=$(vsm config show) || { pause_if_interactive; return 1; }
  cur_domain=$(jq -r '.domain | gsub("^\\s+|\\s+$"; "")' <<<"$current") || return 1
  acme_port=$(jq -r '.server.acme_port' <<<"$current") || return 1
  https_port=$(jq -r '.server.https_port' <<<"$current") || return 1

  if is_ip_host "$cur_domain"; then
    printf '当前状态: 未配置域名（明文 HTTP 模式，存在泄露风险）\n' >&2
  else
    printf '当前域名: %s（HTTPS 加密模式）\n' "$cur_domain" >&2
  fi

  printf '\n'
  menu_item 1 '绑定 / 更换域名并启用 HTTPS'
  menu_item 2 '同步或续期当前证书'
  menu_item 0 '返回主菜单'
  choose 2 '选择域名操作' 1 || return 0
  case $answer in
    1)
      ask '输入已解析到本机的域名（用于启用 HTTPS）' "$cur_domain" || return 0
      new_domain=$answer
      if is_ip_host "$new_domain"; then
        printf '请输入有效域名（不能为纯 IP），以签发 TLS 证书并启用 HTTPS。\n' >&2
        pause_if_interactive
        return 1
      fi
      ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 0
      email=$answer
      if ! candidate=$(vsm config prepare --domain="$new_domain" --ports=keep); then
        pause_if_interactive
        return 1
      fi
      mode=$(jq -r '.mode' <<<"$candidate") || return 1
      sni=$(jq -r '.tls_sni' <<<"$candidate") || return 1
      acme_port=$(jq -r '.config.server.acme_port' <<<"$candidate") || return 1
      https_port=$(jq -r '.config.server.https_port' <<<"$candidate") || return 1
      printf '订阅模式：%s  订阅域名：%s  (https://%s/sub/...)\n' "$mode" "$sni" "$sni" >&2
      confirm '确认保存域名配置、签发/同步 TLS 证书并启用 HTTPS' 1 || return 0
      if [[ $root == / ]]; then
        "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
        "$script_dir/cert.sh" issue "$sni" "$email" || rc=$?
      fi
      if ((rc == 0)); then
        jq -c '.config' <<<"$candidate" | vsm config write --replace || rc=$?
      fi
      if ((rc == 0)); then
        vsm apply || rc=$?
      fi
      pause_if_interactive
      return "$rc"
      ;;
    2)
      if is_ip_host "$cur_domain"; then
        printf '尚未配置域名，请先选择 1 绑定域名。\n' >&2
        pause_if_interactive
        return 1
      fi
      confirm "确认检查并续期域名证书 ($cur_domain)" 2 || return 0
      if [[ $root == / ]]; then
        "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
        "$script_dir/cert.sh" renew "$cur_domain" || rc=$?
        pause_if_interactive
        return "$rc"
      fi
      ;;
  esac
}

manage_certificate() {
  require_initialized || return 1
  local config domain sni email acme_port https_port last=1 rc=0
  menu_heading '证书管理'
  config=$(vsm config show) || { pause_if_interactive; return 1; }
  domain=$(jq -r '.domain | gsub("^\\s+|\\s+$"; "")' <<<"$config") || return 1
  acme_port=$(jq -r '.server.acme_port' <<<"$config") || return 1
  https_port=$(jq -r '.server.https_port' <<<"$config") || return 1
  if is_ip_host "$domain"; then
    sni=www.bing.com
  else
    sni=$domain
  fi
  printf '当前证书名称：%s\n' "$sni"
  bash "$script_dir/cert.sh" status --root "$root" "$sni" || true
  printf '\n'
  if ! is_ip_host "$domain"; then
    menu_item 1 '续期当前域名证书'
    menu_item 2 '同步已有 ACME 证书'
    menu_item 3 "申请或复用 Let's Encrypt 证书"
    local auto_active=0 auto_label='启用自动续期 (systemd timer)'
    if [[ $root == / ]] && command -v systemctl >/dev/null 2>&1 \
      && systemctl is-active --quiet vps-sub-meter-cert-renew.timer 2>/dev/null; then
      auto_active=1
      auto_label='停用自动续期 (systemd timer)'
    fi
    menu_item 4 "$auto_label"
    last=4
  else
    menu_item 1 '检查并补齐 IP 模式的自签证书'
  fi
  menu_item 0 '返回主菜单'
  choose "$last" '选择证书操作' 0 || return 0
  case $answer in
    1)
      if ! is_ip_host "$domain"; then
        confirm '确认检查并续期证书' 2 || return 0
        if [[ $root == / ]]; then
          "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
          bash "$script_dir/cert.sh" renew "$sni" || rc=$?
          pause_if_interactive
          return "$rc"
        fi
      else
        confirm '确认检查并补齐自签证书' 2 || return 0
        if [[ $root == / ]]; then
          bash "$script_dir/cert.sh" self-signed "$sni" || rc=$?
          if ((rc == 0)); then vsm apply || rc=$?; fi
          if ((rc == 0)); then bash "$script_dir/cert.sh" reload "$sni" || rc=$?; fi
          pause_if_interactive
          return "$rc"
        fi
      fi
      ;;
    2)
      confirm '确认同步已有证书' 2 || return 0
      if [[ $root == / ]]; then
        bash "$script_dir/cert.sh" sync "$sni" || rc=$?
        pause_if_interactive
        return "$rc"
      fi
      ;;
    3)
      ask '证书账户邮箱（首次安装 acme.sh 时使用，回车跳过）' '' || return 0
      email=$answer
      confirm "确认申请或复用 Let's Encrypt 证书" 2 || return 0
      if [[ $root == / ]]; then
        "$script_dir/net.sh" sync-ufw "${acme_port}/tcp" "${https_port}/tcp" || true
        bash "$script_dir/cert.sh" issue "$sni" "$email" || rc=$?
        pause_if_interactive
        return "$rc"
      fi
      ;;
    4)
      if [[ $root != / ]]; then
        printf '沙箱模式下不支持修改宿主机 systemd 定时器。\n' >&2
        pause_if_interactive
        return 0
      fi
      if ((auto_active)); then
        confirm '确认停用证书自动续期定时器' 2 || return 0
        bash "$script_dir/cert.sh" auto-disable || rc=$?
      else
        confirm '确认启用证书自动续期定时器' 1 || return 0
        bash "$script_dir/cert.sh" auto-enable || rc=$?
      fi
      pause_if_interactive
      return "$rc"
      ;;
  esac
}

rotate_keys() {
  require_initialized || return 1
  confirm '确认轮换全部节点密钥，客户端需刷新订阅' 2 || return 0
  local rc=0
  vsm keys rotate --yes || rc=$?
  pause_if_interactive
  return "$rc"
}

apply_config() {
  require_initialized || return 1
  local rc=0
  vsm apply || rc=$?
  pause_if_interactive
  return "$rc"
}

manage_traffic() {
  require_initialized || return 1
  local current settings quota day time reset_tz display_tz upload download args=() rc=0

  menu_heading '流量账期与校准'
  menu_item 1 '设置月额度、重置时间与时区'
  menu_item 2 '校准已用上传和下载流量'
  menu_item 3 '立即保存当前流量账本'
  menu_item 0 '返回主菜单'
  choose 3 '选择流量操作' 0 || return 0
  case $answer in
    1)
      current=$(vsm config show) || { pause_if_interactive; return 1; }
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
        pause_if_interactive
        return 0
      fi
      confirm '确认保存流量设置' 2 || return 0
      vsm traffic configure "${args[@]}" || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
    2)
      ask '已用客户端上传 (GiB，清零请输入 0.0)' || return 0
      upload=$answer
      ask '已用客户端下载 (GiB，清零请输入 0.0)' || return 0
      download=$answer
      confirm '确认覆盖本期已用上传和下载流量' 2 || return 0
      vsm traffic set -- "$upload" "$download" || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
    3)
      confirm '确认立即保存流量账本' 2 || return 0
      vsm traffic checkpoint || rc=$?
      pause_if_interactive
      return "$rc"
      ;;
  esac
}

main() {
  if ! command -v jq >/dev/null 2>&1; then
    printf '找不到 jq，请安装后再使用 ly。\n' >&2
    exit 2
  fi

  if "$interactive"; then
    trap cleanup EXIT
    trap 'cleanup; exit 130' INT TERM HUP
    trap 'winch_resized=1' WINCH
    enter_alt_screen
  fi

  local need_collect=1 force_ver=0 choice_rc=0
  while true; do
    if "$interactive"; then
      if ((need_collect)); then
        collect_dashboard_data "$force_ver"
        need_collect=0
        force_ver=0
      fi
      render_main_menu
      read_main_choice
      choice_rc=$?
      case $choice_rc in
        1) exit 0 ;;
        2) continue ;;
        3) need_collect=1; force_ver=1; continue ;;
      esac
      need_collect=1
    else
      render_main_menu
      choose 13 '请输入编号（0–13）' || exit 0
    fi

    case $answer in
      1) initialize ;;
      2) subscriptions ;;
      3) tokens ;;
      4) rotate_keys ;;
      5) update_cores ;;
      6) manage_domain ;;
      7) manage_argo ;;
      8) apply_config ;;
      9) uninstall_vsm ;;
      10) manage_traffic ;;
      11) manage_ports ;;
      12) manage_certificate ;;
      13) view_logs ;;
    esac
  done
}

main
