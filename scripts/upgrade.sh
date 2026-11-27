#!/usr/bin/env bash
{
set -euo pipefail

# Universal idempotent upgrade entrypoint for ALL versions:
#   - Gen 1: v1/v2 `sub-srv` + Caddy + `sing-box-yg` (/etc/sub-srv/config.conf)
#   - Gen 2: `sbctl` v0.0.1..v0.0.28 (/etc/sbctl/config.toml) + hybrid PQC/Argo
#   - Gen 3: `vps-sub-meter` v2.0.0+ (/etc/vps-sub-meter/config.toml)
#
# Signature placeholder replaced by prepare-installer.py in signed releases.
VSM_PUBLIC_KEY_PEM='@VSM_RELEASE_PUBLIC_KEY_PEM@'
REPO_SLUG="xiaolingxiaoying/vps-sub-meter"

usage() {
  cat <<'EOF'
用法:
  sudo bash upgrade.sh [选项]

选项:
  --check             仅检查当前部署版本与迁移兼容性，不修改任何文件或服务
  --local-dir <dir>   从本地构建目录升级（需包含 vps-sub-meter、scripts/、templates/）
  --from-source       从 GitHub 仓库拉取源码构建并升级（无需等待预编译 Release）
  --branch <name>     配合 --from-source 指定分支（默认 dev）
  --release-only     仅使用已签名的发行包，不回退为源码构建
  --force-templates   强制用新版覆盖 /etc/vps-sub-meter/templates/*
  -h, --help          显示此帮助
EOF
}

CHECK_ONLY=0
LOCAL_DIR=""
FROM_SOURCE=0
SOURCE_BRANCH="dev"
FORCE_TEMPLATES=0
RELEASE_ONLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=1; shift ;;
    --local-dir) LOCAL_DIR=${2:?missing dir}; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    --branch) SOURCE_BRANCH=${2:?missing branch}; shift 2 ;;
    --force-templates) FORCE_TEMPLATES=1; shift ;;
    --release-only) RELEASE_ONLY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$RELEASE_ONLY" -eq 1 && ( -n "$LOCAL_DIR" || "$FROM_SOURCE" -eq 1 ) ]]; then
  echo "--release-only 不能用于本地构建或源码升级。" >&2
  exit 2
fi

detect_local_generation() {
  if [[ -f /etc/vps-sub-meter/config.toml && -f /var/lib/vps-sub-meter/current.json ]]; then
    echo "v2"
  elif [[ -f /etc/sbctl/config.toml ]]; then
    echo "sbctl"
  elif [[ -f /etc/sub-srv/config.conf ]]; then
    echo "sub-srv-v1"
  elif [[ -f /etc/vps-sub-meter/config.toml ]]; then
    echo "v2"
  else
    echo "none"
  fi
}

check_format_iec() {
  awk -v b="${1:-0}" 'BEGIN {
    if (b >= 1099511627776) printf "%.2f TiB", b/1099511627776;
    else if (b >= 1073741824) printf "%.2f GiB", b/1073741824;
    else if (b >= 1048576) printf "%.2f MiB", b/1048576;
    else if (b >= 1024) printf "%.2f KiB", b/1024;
    else printf "%.0f B", b;
  }'
}

check_svc_state() {
  local unit=$1 st
  if ! command -v systemctl >/dev/null 2>&1; then
    printf '未知'
    return 0
  fi
  st=$(systemctl is-active "$unit" 2>/dev/null || true)
  case "$st" in
    active) printf '运行中' ;;
    activating|reloading) printf '启动中' ;;
    failed) printf '异常' ;;
    inactive|dead|'') printf '未运行' ;;
    *) printf '%s' "$st" ;;
  esac
}

check_toml_val() {
  local file=$1 key=$2
  [[ -r "$file" ]] || return 0
  awk -F'=' -v k="$key" '
    $1 ~ "^[[:space:]]*" k "[[:space:]]*$" {
      v=$2
      sub(/#.*/, "", v)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      gsub(/^"|"$|^'\''|'\''$/, "", v)
      print v
      exit
    }' "$file" 2>/dev/null || true
}

run_upgrade_check() {
  local use_color=0
  if [[ -t 1 && ${TERM:-dumb} != dumb && -z ${NO_COLOR:-} ]]; then
    use_color=1
  fi
  c_paint() {
    if ((use_color)); then
      printf '\033[%sm%s\033[0m' "$1" "$2"
    else
      printf '%s' "$2"
    fi
  }

  local py_out="" py_rc=0 gen="" domain="" public_ip="" interface="" tokens_preserved=""
  local argo_enabled="" extra_https_port="" acc_rx="" acc_tx=""
  if [[ -r /usr/local/lib/vps-sub-meter/migrate_legacy.py ]] && command -v python3 >/dev/null 2>&1; then
    set +e
    py_out=$(python3 /usr/local/lib/vps-sub-meter/migrate_legacy.py --root / --check --json 2>/dev/null)
    py_rc=$?
    if [[ "$py_rc" -ne 0 && "$py_rc" -ne 3 ]]; then
      py_out=$(python3 /usr/local/lib/vps-sub-meter/migrate_legacy.py --root / --check 2>/dev/null)
      py_rc=$?
    fi
    set -e
    if [[ "$py_rc" -eq 3 ]]; then
      echo "未检测到任何历史或现行部署（/etc/vps-sub-meter、/etc/sbctl 或 /etc/sub-srv），请使用 install.sh 进行全新安装。" >&2
      exit 3
    fi
    if [[ "$py_rc" -eq 0 && -n "$py_out" ]]; then
      local parsed=""
      if parsed=$(python3 -c '
import json, sys
try:
    d = json.loads(sys.stdin.read())
except Exception:
    raise SystemExit(1)
if not isinstance(d, dict) or not d.get("generation"):
    raise SystemExit(1)
def clean(v):
    if v is None: return ""
    if isinstance(v, bool): return "true" if v else "false"
    return str(v).replace("\x1f", " ").replace("\n", " ").replace("\r", "")
fields = [
    d.get("generation", ""),
    d.get("domain", ""),
    d.get("public_ip", ""),
    d.get("interface", ""),
    d.get("tokens_preserved", ""),
    d.get("argo_enabled", ""),
    d.get("extra_https_port", ""),
    d.get("traffic_acc_rx", ""),
    d.get("traffic_acc_tx", ""),
]
print("\x1f".join(clean(x) for x in fields))
' <<<"$py_out" 2>/dev/null); then
        IFS=$'\x1f' read -r gen domain public_ip interface tokens_preserved argo_enabled extra_https_port acc_rx acc_tx <<<"$parsed"
      fi
    fi
  fi

  if [[ -z "$gen" ]]; then
    gen=$(detect_local_generation)
  fi
  if [[ "$gen" == "none" ]]; then
    echo "未检测到任何历史或现行部署（/etc/vps-sub-meter、/etc/sbctl 或 /etc/sub-srv），请使用 install.sh 进行全新安装。" >&2
    exit 3
  fi

  local gen_label="" cfg_file="" conclusion=""
  case "$gen" in
    v2)
      gen_label="vps-sub-meter v2（现行架构）"
      cfg_file="/etc/vps-sub-meter/config.toml"
      conclusion="检查通过：当前为现行 v2 部署，支持就地平滑升级。"
      ;;
    sbctl)
      gen_label="sbctl 历史部署（Gen 2）"
      cfg_file="/etc/sbctl/config.toml"
      conclusion="检查通过：支持从 sbctl 事务化平滑迁移至 vps-sub-meter v2。"
      ;;
    sub-srv-v1)
      gen_label="sub-srv 早期部署（Gen 1）"
      cfg_file="/etc/sub-srv/config.conf"
      conclusion="检查通过：支持从 sub-srv 自动迁移配置、凭据与流量账本至 v2。"
      ;;
    *)
      gen_label="$gen"
      cfg_file="未知"
      conclusion="检查通过：支持平滑升级。"
      ;;
  esac

  local cur_vsm="" latest_vsm="" latest_note="" eff_url=""
  if [[ -x /usr/local/bin/vps-sub-meter ]]; then
    cur_vsm=$(/usr/local/bin/vps-sub-meter --version 2>/dev/null | awk '{print $NF; exit}' || true)
    cur_vsm=${cur_vsm#v}
  fi
  if command -v curl >/dev/null 2>&1; then
    eff_url=$(curl -fsSLI --connect-timeout 3 --max-time 5 -o /dev/null -w '%{url_effective}' \
      "https://github.com/${REPO_SLUG}/releases/latest" 2>/dev/null || true)
    if [[ "$eff_url" == */releases/tag/* ]]; then
      latest_vsm=${eff_url##*/releases/tag/}
      latest_vsm=${latest_vsm%%\?*}
      latest_vsm=${latest_vsm#v}
    fi
  fi
  if [[ -n "$latest_vsm" && -n "$cur_vsm" ]]; then
    if [[ "$latest_vsm" == "$cur_vsm" ]]; then
      latest_note="$(c_paint 32 "v${latest_vsm}（已是最新 Release）")"
    elif [[ "$(printf '%s\n%s\n' "$latest_vsm" "$cur_vsm" | sort -V 2>/dev/null | tail -n1)" == "$latest_vsm" ]]; then
      latest_note="$(c_paint '1;33' "v${latest_vsm}（发现新版本，可升级）")"
    else
      latest_note="v${latest_vsm}（当前版本高于最新 Release）"
    fi
  elif [[ -n "$latest_vsm" ]]; then
    latest_note="v${latest_vsm}"
  else
    latest_note="$(c_paint 33 '暂未检测到已签名 Release（升级时将自动从 dev 分支源码编译）')"
  fi

  echo ""
  c_paint '1;36' "================================================"
  echo ""
  c_paint '1;36' "  vps-sub-meter 升级与迁移检查（只读）"
  echo ""
  c_paint '1;36' "================================================"
  echo ""
  echo ""
  c_paint '1;36' "[当前部署]"
  echo ""
  printf '  部署代际：%s\n' "$gen_label"
  printf '  配置文件：%s\n' "$cfg_file"

  if [[ "$gen" == "v2" ]]; then
    local status_json="" sub_addr="" sub_proto="" iface="" traffic_line="" gen_id="" argo_line=""
    local xray_ver="未安装" sb_ver="未安装" xray_st sb_st vsm_st
    if [[ -x /usr/local/bin/vps-sub-meter ]]; then
      status_json=$(/usr/local/bin/vps-sub-meter --root / status --json 2>/dev/null || true)
    fi
    if [[ -n "$status_json" ]] && command -v python3 >/dev/null 2>&1; then
      local s_parsed=""
      if s_parsed=$(python3 -c '
import json, sys
try:
    s = json.loads(sys.stdin.read())
except Exception:
    raise SystemExit(1)
t = s.get("traffic") or {}
g = s.get("generation") or {}
print("\x1f".join(str(x).replace("\x1f", " ") for x in [
    s.get("address", ""),
    "true" if s.get("https_enabled") else "false",
    s.get("interface", ""),
    t.get("upload_bytes", 0),
    t.get("download_bytes", 0),
    t.get("total_used_bytes", 0),
    t.get("monthly_limit_bytes", 0),
    g.get("id", ""),
    len(g.get("direct_nodes") or []),
    "true" if s.get("argo_enabled") else "false",
]))
' <<<"$status_json" 2>/dev/null); then
        local https_on up_b down_b tot_b lim_b node_cnt argo_on
        IFS=$'\x1f' read -r sub_addr https_on iface up_b down_b tot_b lim_b gen_id node_cnt argo_on <<<"$s_parsed"
        if [[ "$https_on" == "true" ]]; then
          sub_proto="https://${sub_addr} (HTTPS)"
        else
          sub_proto="http://${sub_addr} (HTTP 明文)"
        fi
        local lim_disp
        lim_disp=$( [[ "$lim_b" =~ ^[0-9]+$ && "$lim_b" -gt 0 ]] && check_format_iec "$lim_b" || printf '不限' )
        traffic_line="$(check_format_iec "$tot_b") / ${lim_disp}（上传 $(check_format_iec "$up_b") / 下载 $(check_format_iec "$down_b")）"
        if [[ "$argo_on" == "true" ]]; then
          argo_line="已启用 [$(check_svc_state vps-sub-meter-argo.service)]"
        else
          argo_line="未启用"
        fi
      fi
    fi

    if [[ -z "$sub_proto" && -r /etc/vps-sub-meter/config.toml ]]; then
      local cfg_dom cfg_ip cfg_if
      cfg_dom=$(check_toml_val /etc/vps-sub-meter/config.toml domain)
      cfg_ip=$(check_toml_val /etc/vps-sub-meter/config.toml public_ip)
      cfg_if=$(check_toml_val /etc/vps-sub-meter/config.toml interface)
      iface=${iface:-$cfg_if}
      if [[ -n "$cfg_dom" ]]; then
        sub_proto="https://${cfg_dom} (HTTPS)"
      elif [[ -n "$cfg_ip" ]]; then
        sub_proto="http://${cfg_ip} (HTTP 明文)"
      fi
    fi

    if [[ -x /usr/local/bin/xray ]]; then
      xray_ver=$(/usr/local/bin/xray version 2>/dev/null | awk 'NR==1{print "v" $2}' || echo "已安装")
    fi
    if [[ -x /usr/local/bin/sing-box ]]; then
      sb_ver=$(/usr/local/bin/sing-box version 2>/dev/null | awk 'NR==1{print "v" $3}' || echo "已安装")
    fi
    vsm_st=$(check_svc_state vps-sub-meter.service)
    xray_st=$(check_svc_state vps-sub-meter-xray.service)
    sb_st=$(check_svc_state vps-sub-meter-singbox.service)

    printf '  当前版本：%s\n' "${cur_vsm:+v${cur_vsm} [${vsm_st}]}"
    printf '  最新版本：%s\n' "$latest_note"
    [[ -n "$sub_proto" ]] && printf '  订阅地址：%s\n' "$sub_proto"
    [[ -n "$iface" ]] && printf '  监控网卡：%s\n' "$iface"
    [[ -n "$traffic_line" ]] && printf '  本期流量：%s\n' "$traffic_line"
    if [[ -n "$gen_id" ]]; then
      printf '  当前批次：%s (%s 个直连节点)\n' "$gen_id" "${node_cnt:-0}"
    fi
    [[ -n "$argo_line" ]] && printf '  Argo 隧道：%s\n' "$argo_line"
    printf '  代理内核：Xray %s [%s]  |  sing-box %s [%s]\n' "$xray_ver" "$xray_st" "$sb_ver" "$sb_st"
  else
    printf '  目标版本：%s\n' "$latest_note"
    if [[ -n "$domain" ]]; then
      printf '  订阅域名：%s (HTTPS)\n' "$domain"
    elif [[ -n "$public_ip" ]]; then
      printf '  公网地址：%s (IP / HTTP 模式)\n' "$public_ip"
    fi
    [[ -n "$interface" ]] && printf '  监控网卡：%s\n' "$interface"
    [[ -n "$tokens_preserved" ]] && printf '  保留凭据：%s 个有效订阅凭据\n' "$tokens_preserved"
    if [[ "$acc_rx" =~ ^[0-9]+$ && "$acc_tx" =~ ^[0-9]+$ ]]; then
      printf '  历史流量：上传 %s / 下载 %s（自动继承）\n' "$(check_format_iec "$acc_rx")" "$(check_format_iec "$acc_tx")"
    fi
    if [[ "$argo_enabled" == "true" ]]; then
      printf '  Argo 隧道：已启用（将自动迁移隧道配置与 Token）\n'
    elif [[ -n "$argo_enabled" ]]; then
      printf '  Argo 隧道：未启用\n'
    fi
    [[ -n "$extra_https_port" ]] && printf '  额外端口：%s\n' "$extra_https_port"
  fi

  echo ""
  c_paint '1;36' "[升级 / 迁移计划]"
  echo ""
  echo "  1. 升级前自动备份现有配置与账本至 /root/pre-vsm-upgrade-<UTC>.tar.gz"
  if [[ "$gen" == "v2" ]]; then
    echo "  2. 更新主程序、交互菜单 (ly / vsm) 与运维脚本库，默认保留现有配置与模板"
  else
    echo "  2. 安装 vps-sub-meter v2 并事务化迁移节点密钥、订阅凭据、证书与流量账本"
  fi
  echo "  3. 检查官方 Xray / sing-box 内核版本及 BBR / UDP 系统调优参数"
  echo "  4. 执行 vsm apply 双内核校验配置并平滑重启服务，随后停用旧版残留单元"

  echo ""
  c_paint '1;36' "[检查结论]"
  echo ""
  echo "  $(c_paint 32 "$conclusion")"
  echo "  本次仅执行只读检查，未修改任何文件或运行中的服务。"
  echo ""
  echo "  执行升级："
  echo "    curl -fsSL https://raw.githubusercontent.com/${REPO_SLUG}/dev/scripts/upgrade.sh | sudo bash"
  echo ""
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  run_upgrade_check
  exit 0
fi

if [[ "${EUID}" -ne 0 ]]; then
  echo "请使用 sudo / root 运行 upgrade.sh" >&2
  exit 2
fi

if [[ ! -r /etc/os-release ]]; then
  echo "无法识别操作系统" >&2
  exit 2
fi
. /etc/os-release
case "${ID}" in
  debian|ubuntu) ;;
  *) echo "仅支持 Debian 或 Ubuntu" >&2; exit 2 ;;
esac

EXISTING_GEN=$(detect_local_generation)
if [[ "$EXISTING_GEN" == "none" ]]; then
  echo "未检测到任何历史或现行部署（/etc/vps-sub-meter、/etc/sbctl 或 /etc/sub-srv）。请使用 install.sh 全新安装。" >&2
  exit 3
fi

# Delegate to signed Release upgrade.sh when run directly from raw git branch
# without embedded public key and without explicit --local-dir / --from-source.
if [[ -z "$LOCAL_DIR" && "$FROM_SOURCE" -eq 0 && "$VSM_PUBLIC_KEY_PEM" == @* ]]; then
  release_url="https://github.com/${REPO_SLUG}/releases/latest/download/upgrade.sh"
  tmp_dl=$(mktemp -d)
  if curl -fsSL --proto '=https' --connect-timeout 8 --max-time 30 \
      -o "$tmp_dl/upgrade.sh" "$release_url" 2>/dev/null \
      && grep -q 'BEGIN PUBLIC KEY' "$tmp_dl/upgrade.sh"; then
    pass_flags=()
    [[ "$FORCE_TEMPLATES" -eq 1 ]] && pass_flags+=(--force-templates)
    bash "$tmp_dl/upgrade.sh" "${pass_flags[@]}"
    rm -rf "$tmp_dl"
    exit 0
  fi
  rm -rf "$tmp_dl"
  if [[ "$RELEASE_ONLY" -eq 1 ]]; then
    echo "找不到已签名的发行包，未执行升级。" >&2
    exit 2
  fi
  echo "未检测到已签名的最新 Release upgrade.sh，自动回退为源码构建升级（分支: ${SOURCE_BRANCH}）..."
  FROM_SOURCE=1
fi

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  ca-certificates curl jq openssl socat tar unzip nftables >/dev/null

WORK_DIR=$(mktemp -d)
trap 'rm -rf -- "$WORK_DIR"' EXIT

STAGE_DIR=""
if [[ -n "$LOCAL_DIR" ]]; then
  STAGE_DIR="$LOCAL_DIR"
elif [[ "$FROM_SOURCE" -eq 1 ]]; then
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential git pkg-config >/dev/null
  if ! command -v cargo >/dev/null 2>&1; then
    export RUSTUP_HOME="$WORK_DIR/rustup"
    export CARGO_HOME="$WORK_DIR/cargo"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --no-modify-path >/dev/null
    export PATH="$CARGO_HOME/bin:$PATH"
  fi
  git clone --depth 1 --branch "$SOURCE_BRANCH" "https://github.com/${REPO_SLUG}.git" "$WORK_DIR/src"
  (
    cd "$WORK_DIR/src"
    CARGO_TARGET_DIR="$WORK_DIR/target" cargo build --release --locked --no-default-features
  )
  STAGE_DIR="$WORK_DIR/stage"
  mkdir -p "$STAGE_DIR"
  install -m 0755 "$WORK_DIR/target/release/vps-sub-meter" "$STAGE_DIR/vps-sub-meter"
  cp -r "$WORK_DIR/src/scripts" "$WORK_DIR/src/templates" "$STAGE_DIR/"
else
  case "$(dpkg --print-architecture)" in
    amd64|arm64) arch=$(dpkg --print-architecture) ;;
    *) echo "仅支持 amd64 和 arm64" >&2; exit 2 ;;
  esac
  manifest_url=${VSM_MANIFEST_URL:-"https://github.com/${REPO_SLUG}/releases/latest/download/manifest-${arch}.json"}
  curl --fail --globoff --location --silent --show-error "$manifest_url" >"$WORK_DIR/manifest.json"
  jq -S -c 'del(.signature)' "$WORK_DIR/manifest.json" | tr -d '\r\n' >"$WORK_DIR/canonical.json"
  jq -r '.signature' "$WORK_DIR/manifest.json" | base64 -d >"$WORK_DIR/signature.bin"
  printf '%s\n' "$VSM_PUBLIC_KEY_PEM" >"$WORK_DIR/public-key.pem"

  if ! openssl pkeyutl -verify -pubin -inkey "$WORK_DIR/public-key.pem" -rawin \
    -in "$WORK_DIR/canonical.json" -sigfile "$WORK_DIR/signature.bin" >/dev/null 2>&1; then
    echo "release manifest 签名校验失败，已中止升级。" >&2
    exit 2
  fi

  schema=$(jq -er '.schema' "$WORK_DIR/manifest.json")
  [[ "$schema" == 1 ]] || { echo "不支持的 manifest schema: $schema" >&2; exit 2; }

  bin_url=$(jq -er '.vps_sub_meter.url' "$WORK_DIR/manifest.json")
  bin_sha=$(jq -er '.vps_sub_meter.sha256' "$WORK_DIR/manifest.json")
  bundle_url=$(jq -r '.bundle.url // empty' "$WORK_DIR/manifest.json")
  bundle_sha=$(jq -r '.bundle.sha256 // empty' "$WORK_DIR/manifest.json")

  curl --fail --location --silent --show-error "$bin_url" >"$WORK_DIR/vps-sub-meter"
  printf '%s  %s\n' "$bin_sha" "$WORK_DIR/vps-sub-meter" | sha256sum --check --status
  chmod 0755 "$WORK_DIR/vps-sub-meter"

  if [[ -n "$bundle_url" && -n "$bundle_sha" ]]; then
    curl --fail --location --silent --show-error "$bundle_url" >"$WORK_DIR/bundle.tar.gz"
    printf '%s  %s\n' "$bundle_sha" "$WORK_DIR/bundle.tar.gz" | sha256sum --check --status
    tar -xzf "$WORK_DIR/bundle.tar.gz" -C "$WORK_DIR"
  fi
  STAGE_DIR="$WORK_DIR"
fi

# 1. Full encrypted-permission pre-upgrade backup before mutating system files
BACKUP="/root/pre-vsm-upgrade-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
BACKUP_PATHS=()
for p in /etc/vps-sub-meter /var/lib/vps-sub-meter /etc/sbctl /var/lib/sbctl \
         /etc/sing-box /usr/local/etc/xray /usr/local/etc/sing-box-pqc \
         /usr/local/etc/xray-sub /var/lib/xray-sub /etc/*argo-sub \
         /etc/sub-srv /var/lib/subsrv /etc/s-box /etc/letsencrypt/renewal; do
  [[ -e "$p" ]] && BACKUP_PATHS+=("$p")
done
if [[ ${#BACKUP_PATHS[@]} -gt 0 ]]; then
  umask 077
  tar -czf "$BACKUP" "${BACKUP_PATHS[@]}" 2>/dev/null || true
  echo "已创建升级前配置与账本备份: $BACKUP"
fi

# 2. Install binaries, modular scripts & templates (preserving existing v2 user config/templates unless forced)
INSTALL_FLAGS=(--local-dir "$STAGE_DIR")
if [[ "$FORCE_TEMPLATES" -eq 1 || "$EXISTING_GEN" != "v2" ]]; then
  INSTALL_FLAGS+=(--force-templates)
fi
bash "$STAGE_DIR/scripts/install.sh" "${INSTALL_FLAGS[@]}"

# 3. Ensure official Xray + sing-box cores and BBR/UDP sysctl tuning
bash /usr/local/lib/vps-sub-meter/cores.sh ensure
bash /usr/local/lib/vps-sub-meter/net.sh tune-sysctl || true

# 4. Run non-destructive configuration & traffic state migration for legacy versions
if [[ "$EXISTING_GEN" != "v2" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 >/dev/null
  fi
  python3 /usr/local/lib/vps-sub-meter/migrate_legacy.py --root /
  /usr/local/bin/vsm status >/dev/null
fi

# 5. Reconcile certificates & cleanly retire Gen 1 / Gen 2 services before `vsm apply`
DOMAIN_VAL=$(awk -F'"' '/^domain[[:space:]]*=/{print $2; exit}' /etc/vps-sub-meter/config.toml || true)
PUBLIC_IP_VAL=$(awk -F'"' '/^public_ip[[:space:]]*=/{print $2; exit}' /etc/vps-sub-meter/config.toml || true)
LEGACY_CERT_MODE=""
if [[ -r /etc/sbctl/config.toml ]]; then
  LEGACY_CERT_MODE=$(awk -F'"' '/^certificate_mode[[:space:]]*=/{print $2; exit}' /etc/sbctl/config.toml || true)
fi

cert_pair_valid() {
  local sni=$1
  local dir="/var/lib/vps-sub-meter/certificates/${sni}"
  [[ -s "${dir}/fullchain.pem" && -s "${dir}/privkey.pem" ]] || return 1
  local p1 p2
  p1=$(openssl x509 -in "${dir}/fullchain.pem" -noout -pubkey 2>/dev/null | openssl md5 2>/dev/null || true)
  p2=$(openssl pkey -in "${dir}/privkey.pem" -pubout 2>/dev/null | openssl md5 2>/dev/null || true)
  [[ -n "$p1" && "$p1" == "$p2" ]]
}

import_legacy_sbctl_cert() {
  local sni=$1
  local src="/var/lib/sbctl/certificates/${sni}"
  local dst="/var/lib/vps-sub-meter/certificates/${sni}"
  [[ -d "$src" ]] || return 1
  local fc="" pk=""
  if [[ -r "${src}/fullchain.pem" && -r "${src}/privkey.pem" ]]; then
    fc="${src}/fullchain.pem"
    pk="${src}/privkey.pem"
  elif [[ -r "${src}/cert.pem" && -r "${src}/key.pem" ]]; then
    fc="${src}/cert.pem"
    pk="${src}/key.pem"
  else
    return 1
  fi
  getent group vsm-cert >/dev/null || groupadd --system vsm-cert
  mkdir -p "$dst"
  chown root:vsm-cert "$dst"
  chmod 0750 "$dst"
  install -m 0640 -o root -g vsm-cert -L "$fc" "${dst}/fullchain.pem" 2>/dev/null \
    || cat "$fc" > "${dst}/fullchain.pem"
  install -m 0640 -o root -g vsm-cert -L "$pk" "${dst}/privkey.pem" 2>/dev/null \
    || cat "$pk" > "${dst}/privkey.pem"
  chown root:vsm-cert "${dst}/fullchain.pem" "${dst}/privkey.pem"
  chmod 0640 "${dst}/fullchain.pem" "${dst}/privkey.pem"
}

if [[ "$EXISTING_GEN" == "sbctl" ]]; then
  if [[ -n "$DOMAIN_VAL" && -r "/etc/letsencrypt/live/${DOMAIN_VAL}/fullchain.pem" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh sync "$DOMAIN_VAL" || true
  elif [[ -n "$DOMAIN_VAL" ]]; then
    import_legacy_sbctl_cert "$DOMAIN_VAL" || true
  elif [[ -d /var/lib/sbctl/certificates/www.bing.com ]]; then
    import_legacy_sbctl_cert "www.bing.com" || true
  fi
  bash /usr/local/lib/vps-sub-meter/migrate-sbctl.sh
elif [[ "$EXISTING_GEN" == "sub-srv-v1" ]]; then
  # Copy Caddy certificate if present before stopping Caddy
  if [[ -n "$DOMAIN_VAL" ]]; then
    caddy_crt=$(find /var/lib/caddy/.local/share/caddy/certificates -type f -name "${DOMAIN_VAL}.crt" 2>/dev/null | head -n1 || true)
    caddy_key=$(find /var/lib/caddy/.local/share/caddy/certificates -type f -name "${DOMAIN_VAL}.key" 2>/dev/null | head -n1 || true)
    if [[ -r "$caddy_crt" && -r "$caddy_key" ]]; then
      dest="/var/lib/vps-sub-meter/certificates/${DOMAIN_VAL}"
      mkdir -p "$dest"
      chown root:vsm-cert "$dest"
      chmod 0750 "$dest"
      install -m 0640 -o root -g vsm-cert "$caddy_crt" "${dest}/fullchain.pem"
      install -m 0640 -o root -g vsm-cert "$caddy_key" "${dest}/privkey.pem"
    fi
  fi
  for u in sub-server.service refresh-sub-copy.timer refresh-sub-copy.service \
           reset-tx-baseline.timer reset-tx-baseline.service \
           sync-caddy-cert-to-singbox.timer sync-caddy-cert-to-singbox.service \
           caddy.service sb.service sing-box.service; do
    systemctl disable --now "$u" 2>/dev/null || true
  done
fi

if [[ -z "$DOMAIN_VAL" ]]; then
  if ! cert_pair_valid "www.bing.com"; then
    rm -f /var/lib/vps-sub-meter/certificates/www.bing.com/{fullchain,privkey}.pem
    bash /usr/local/lib/vps-sub-meter/cert.sh self-signed "www.bing.com"
  fi
elif ! cert_pair_valid "$DOMAIN_VAL"; then
  rm -f "/var/lib/vps-sub-meter/certificates/${DOMAIN_VAL}/"{fullchain,privkey}.pem
  if [[ -r "/etc/letsencrypt/live/${DOMAIN_VAL}/fullchain.pem" && -r "/etc/letsencrypt/live/${DOMAIN_VAL}/privkey.pem" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh sync "$DOMAIN_VAL" || true
  elif [[ "$LEGACY_CERT_MODE" == "self-signed" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh self-signed "$DOMAIN_VAL"
  else
    if ! bash /usr/local/lib/vps-sub-meter/cert.sh issue "$DOMAIN_VAL"; then
      echo "警告: 自动申请 ACME 证书失败（可能受防火墙 80 端口或 CDN 代理影响），已自动签发临时自签证书以确保服务可用。" >&2
      bash /usr/local/lib/vps-sub-meter/cert.sh self-signed "$DOMAIN_VAL"
    fi
  fi
fi

# Ensure certificate readability by vsm-cert group (xray / sing-box / vps-sub-meter)
chown -R root:vsm-cert /var/lib/vps-sub-meter/certificates
find /var/lib/vps-sub-meter/certificates -type d -exec chmod 0750 {} +
find /var/lib/vps-sub-meter/certificates -type f -exec chmod 0640 {} +

# 6. Apply & verify dual-core configs transactionally
/usr/local/bin/vsm apply
if [[ "$EXISTING_GEN" == sbctl && -s /etc/cloudflared/sbctl-argo.token ]] \
   && /usr/local/bin/vsm config show | jq -e '.argo.enabled' >/dev/null; then
  bash /usr/local/lib/vps-sub-meter/argo.sh setup --token-stdin <<<"$(cat /etc/cloudflared/sbctl-argo.token)"
fi
/usr/local/bin/vsm status
echo "升级完成！可运行 \`vsm sub --qr\` 获取订阅，或运行 \`ly\` 打开交互菜单。"
}
