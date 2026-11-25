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
  --force-templates   强制用新版覆盖 /etc/vps-sub-meter/templates/*
  -h, --help          显示此帮助
EOF
}

CHECK_ONLY=0
LOCAL_DIR=""
FROM_SOURCE=0
SOURCE_BRANCH="dev"
FORCE_TEMPLATES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=1; shift ;;
    --local-dir) LOCAL_DIR=${2:?missing dir}; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    --branch) SOURCE_BRANCH=${2:?missing branch}; shift 2 ;;
    --force-templates) FORCE_TEMPLATES=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
  esac
done

detect_local_generation() {
  if [[ -f /etc/vps-sub-meter/config.toml ]]; then
    echo "v2"
  elif [[ -f /etc/sbctl/config.toml ]]; then
    echo "sbctl"
  elif [[ -f /etc/sub-srv/config.conf ]]; then
    echo "sub-srv-v1"
  else
    echo "none"
  fi
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  if [[ -x /usr/local/lib/vps-sub-meter/migrate_legacy.py ]] && command -v python3 >/dev/null 2>&1; then
    exec python3 /usr/local/lib/vps-sub-meter/migrate_legacy.py --root / --check
  fi
  gen=$(detect_local_generation)
  case "$gen" in
    v2)         echo "检测到当前部署: vps-sub-meter v2 (/etc/vps-sub-meter/config.toml) - 支持就地平滑升级。" ;;
    sbctl)      echo "检测到历史部署: sbctl (/etc/sbctl/config.toml) - 支持事务化迁移至 vps-sub-meter v2。" ;;
    sub-srv-v1) echo "检测到早期部署: sub-srv (/etc/sub-srv/config.conf) - 支持自动迁移配置、Token 与流量账本至 v2。" ;;
    *)          echo "未检测到现有部署，请改用 install.sh 进行全新安装。" >&2; exit 3 ;;
  esac
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
  echo "未检测到已签名的最新 Release upgrade.sh，自动回退为源码构建升级（分支: ${SOURCE_BRANCH}）..."
  FROM_SOURCE=1
fi

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  ca-certificates curl jq openssl python3 tar unzip certbot nftables >/dev/null

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
  python3 /usr/local/lib/vps-sub-meter/migrate_legacy.py --root /
  /usr/local/bin/vsm status >/dev/null
fi

# 5. Reconcile certificates & cleanly retire Gen 1 / Gen 2 services before `vsm apply`
DOMAIN_VAL=$(awk -F'"' '/^domain[[:space:]]*=/{print $2; exit}' /etc/vps-sub-meter/config.toml || true)
PUBLIC_IP_VAL=$(awk -F'"' '/^public_ip[[:space:]]*=/{print $2; exit}' /etc/vps-sub-meter/config.toml || true)

if [[ "$EXISTING_GEN" == "sbctl" ]]; then
  if [[ -n "$DOMAIN_VAL" && -r "/etc/letsencrypt/live/${DOMAIN_VAL}/fullchain.pem" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh sync "$DOMAIN_VAL"
  elif [[ -d /var/lib/sbctl/certificates ]]; then
    cp -a /var/lib/sbctl/certificates/. /var/lib/vps-sub-meter/certificates/ 2>/dev/null || true
  fi
  bash /usr/local/lib/vps-sub-meter/migrate-sbctl.sh
  if grep -q '^enabled = true' /etc/vps-sub-meter/config.toml \
     && [[ -r /etc/cloudflared/sbctl-argo.token ]]; then
    bash /usr/local/lib/vps-sub-meter/argo.sh setup "$(cat /etc/cloudflared/sbctl-argo.token)"
  fi
elif [[ "$EXISTING_GEN" == "sub-srv-v1" ]]; then
  # Copy Caddy certificate if present before stopping Caddy
  if [[ -n "$DOMAIN_VAL" ]]; then
    caddy_crt=$(find /var/lib/caddy/.local/share/caddy/certificates -type f -name "${DOMAIN_VAL}.crt" 2>/dev/null | head -n1 || true)
    caddy_key=$(find /var/lib/caddy/.local/share/caddy/certificates -type f -name "${DOMAIN_VAL}.key" 2>/dev/null | head -n1 || true)
    if [[ -r "$caddy_crt" && -r "$caddy_key" ]]; then
      dest="/var/lib/vps-sub-meter/certificates/${DOMAIN_VAL}"
      mkdir -p "$dest"
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
  if [[ -n "$DOMAIN_VAL" && ! -s "/var/lib/vps-sub-meter/certificates/${DOMAIN_VAL}/fullchain.pem" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh issue "$DOMAIN_VAL"
  fi
fi

if [[ -z "$DOMAIN_VAL" ]]; then
  bash /usr/local/lib/vps-sub-meter/cert.sh self-signed "www.bing.com"
elif [[ ! -s "/var/lib/vps-sub-meter/certificates/${DOMAIN_VAL}/fullchain.pem" ]]; then
  if [[ -r "/etc/letsencrypt/live/${DOMAIN_VAL}/fullchain.pem" ]]; then
    bash /usr/local/lib/vps-sub-meter/cert.sh sync "$DOMAIN_VAL"
  else
    bash /usr/local/lib/vps-sub-meter/cert.sh issue "$DOMAIN_VAL"
  fi
fi

# 6. Apply & verify dual-core configs transactionally
/usr/local/bin/vsm apply
/usr/local/bin/vsm status
echo "升级完成！可运行 \`vsm sub --qr\` 获取订阅，或运行 \`ly\` 打开交互菜单。"
}
