#!/usr/bin/env bash
{
set -euo pipefail

# 安全、幂等的 vps-sub-meter 卸载脚本。
# 默认保留 /etc/letsencrypt/live/* 证书，并在清理前自动备份配置与流量账本。

YES=0
KEEP_CONFIG=0
KEEP_CORES=0
NO_BACKUP=0
ROOT="/"

usage() {
  cat <<'EOF'
用法:
  sudo bash uninstall.sh [选项]

选项:
  -y, --yes         跳过交互确认
  --keep-config     保留 /etc/vps-sub-meter 与 /var/lib/vps-sub-meter/traffic-state.json
                    （同时保留系统用户与用户组，且不创建备份）
  --keep-cores      保留 /usr/local/bin/{xray,sing-box,cloudflared} 与 /usr/local/share/xray
                    （同时保留系统用户与用户组）
  --no-backup       彻底卸载时不创建配置与账本备份包
  --root <dir>      指定根目录（默认 /，主要用于沙箱测试）
  -h, --help        显示此帮助
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -y|--yes) YES=1; shift ;;
    --keep-config) KEEP_CONFIG=1; shift ;;
    --keep-cores) KEEP_CORES=1; shift ;;
    --no-backup) NO_BACKUP=1; shift ;;
    --root)
      ROOT=${2:?缺少 --root 参数值}
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

prefix=${ROOT%/}
if [[ "$ROOT" == "/" ]]; then
  prefix=""
  if [[ "${EUID}" -ne 0 ]]; then
    echo "请使用 sudo / root 运行 uninstall.sh" >&2
    exit 2
  fi
fi

print_summary() {
  echo "即将卸载 vps-sub-meter（根目录: ${ROOT}）："
  echo "  - 停止并移除 systemd 服务: vps-sub-meter, vps-sub-meter-xray, vps-sub-meter-singbox, vps-sub-meter-argo"
  echo "  - 清理网络规则与内核调优: nftables (vps_sub_meter_hy2), ufw vps-sub-meter 规则, 99-vps-sub-meter.conf"
  echo "  - 移除程序、软链接与脚本库: vps-sub-meter, vsm, ly, /usr/local/lib/vps-sub-meter"
  echo "  - 移除 certbot deploy hook 与 Argo token"
  if [[ "$KEEP_CORES" -eq 1 ]]; then
    echo "  - [保留] 代理内核: /usr/local/bin/{xray,sing-box,cloudflared} 与 /usr/local/share/xray"
  else
    echo "  - 移除代理内核: /usr/local/bin/{xray,sing-box,cloudflared} 与 /usr/local/share/xray"
  fi
  if [[ "$KEEP_CONFIG" -eq 1 ]]; then
    echo "  - [保留] 配置与流量账本: /etc/vps-sub-meter 与 /var/lib/vps-sub-meter/traffic-state.json"
  elif [[ "$NO_BACKUP" -eq 1 ]]; then
    echo "  - 移除配置与状态目录（按 --no-backup 跳过备份）"
  else
    echo "  - 卸载前自动备份配置与流量账本至 /root/vsm-uninstall-backup-<UTC>.tar.gz，随后清理目录"
  fi
  echo "  - [保留] /etc/letsencrypt/live/* 域名证书（如有）"
}

if [[ "$YES" -eq 0 ]]; then
  print_summary
  answer=""
  if [[ -t 0 ]]; then
    printf '确认卸载 vps-sub-meter？输入 1 确认，0 取消 [0]: ' >&2
    IFS= read -r answer || answer=""
  elif [[ "$ROOT" == "/" ]] && { : </dev/tty; } 2>/dev/null; then
    printf '确认卸载 vps-sub-meter？输入 1 确认，0 取消 [0]: ' >&2
    IFS= read -r answer </dev/tty || answer=""
  elif [[ "$ROOT" != "/" ]]; then
    printf '确认卸载 vps-sub-meter？输入 1 确认，0 取消 [0]: ' >&2
    IFS= read -r answer || answer=""
  else
    echo "非交互环境请追加 --yes 参数确认卸载。" >&2
    exit 2
  fi
  answer=${answer#"${answer%%[![:space:]]*}"}
  answer=${answer%"${answer##*[![:space:]]}"}
  if [[ "$answer" != "1" ]]; then
    echo "已取消卸载。"
    exit 10
  fi
fi

# 1. 卸载前自动备份（保留配置或显式 --no-backup 时跳过）
BACKUP_CREATED=""
if [[ "$KEEP_CONFIG" -eq 0 && "$NO_BACKUP" -eq 0 ]]; then
  backup_rel_paths=()
  [[ -e "${prefix}/etc/vps-sub-meter" || -L "${prefix}/etc/vps-sub-meter" ]] && backup_rel_paths+=("etc/vps-sub-meter")
  [[ -e "${prefix}/var/lib/vps-sub-meter/traffic-state.json" || -L "${prefix}/var/lib/vps-sub-meter/traffic-state.json" ]] \
    && backup_rel_paths+=("var/lib/vps-sub-meter/traffic-state.json")
  if [[ ${#backup_rel_paths[@]} -gt 0 ]]; then
    mkdir -p "${prefix}/root"
    BACKUP_CREATED="${prefix}/root/vsm-uninstall-backup-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
    (
      umask 077
      tar -czf "$BACKUP_CREATED" -C "$ROOT" "${backup_rel_paths[@]}"
    )
    echo "已备份配置与流量账本至: ${BACKUP_CREATED}"
  fi
fi

# 2. 停止并移除 systemd 服务单元
UNITS=(
  "vps-sub-meter.service"
  "vps-sub-meter-xray.service"
  "vps-sub-meter-singbox.service"
  "vps-sub-meter-argo.service"
)
for u in "${UNITS[@]}"; do
  if [[ "$ROOT" == "/" ]] && command -v systemctl >/dev/null 2>&1; then
    systemctl disable --now "$u" 2>/dev/null || true
  fi
  rm -f "${prefix}/etc/systemd/system/${u}"
done
if [[ "$ROOT" == "/" ]] && command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload 2>/dev/null || true
  systemctl reset-failed 2>/dev/null || true
fi

# 3. 清理网络规则与 sysctl
if [[ "$ROOT" == "/" ]] && command -v nft >/dev/null 2>&1; then
  nft delete table inet vps_sub_meter_hy2 2>/dev/null || true
fi
if [[ "$ROOT" == "/" ]] && command -v ufw >/dev/null 2>&1; then
  if ufw status 2>/dev/null | grep -qi '^Status: active'; then
    mapfile -t ufw_nums < <(
      ufw status numbered 2>/dev/null \
        | awk '/vps-sub-meter/ { if (match($0, /\[[[:space:]]*([0-9]+)\]/, m)) print m[1] }' \
        | sort -rn
    )
    for num in "${ufw_nums[@]:-}"; do
      [[ -n "$num" ]] || continue
      ufw --force delete "$num" >/dev/null 2>&1 || true
    done
  fi
fi
if [[ -e "${prefix}/etc/sysctl.d/99-vps-sub-meter.conf" || -L "${prefix}/etc/sysctl.d/99-vps-sub-meter.conf" ]]; then
  rm -f "${prefix}/etc/sysctl.d/99-vps-sub-meter.conf"
  if [[ "$ROOT" == "/" ]] && command -v sysctl >/dev/null 2>&1; then
    sysctl --system >/dev/null 2>&1 || true
  fi
fi

# 4. 清理 certbot deploy hook（绝不删除 /etc/letsencrypt/live/*）与 Argo token
rm -f "${prefix}/etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook"
rm -f "${prefix}/etc/cloudflared/vps-sub-meter-argo.token"
if [[ -d "${prefix}/etc/cloudflared" ]]; then
  rmdir "${prefix}/etc/cloudflared" 2>/dev/null || true
fi

# 5. 删除主程序、快捷命令与运维脚本库
rm -f "${prefix}/usr/local/bin/vps-sub-meter" \
      "${prefix}/usr/local/bin/vsm" \
      "${prefix}/usr/local/bin/ly"
rm -rf "${prefix}/usr/local/lib/vps-sub-meter"

# 6. 清理代理内核
if [[ "$KEEP_CORES" -eq 0 ]]; then
  rm -f "${prefix}/usr/local/bin/xray" \
        "${prefix}/usr/local/bin/sing-box" \
        "${prefix}/usr/local/bin/cloudflared"
  rm -rf "${prefix}/usr/local/share/xray"
fi

# 7. 清理配置与状态目录
if [[ "$KEEP_CONFIG" -eq 1 ]]; then
  rm -f "${prefix}/var/lib/vps-sub-meter/current.json" \
        "${prefix}/var/lib/vps-sub-meter/serve-auth.json" \
        "${prefix}/var/lib/vps-sub-meter/.operation.lock"
  rm -rf "${prefix}/var/lib/vps-sub-meter/generations" \
         "${prefix}/var/lib/vps-sub-meter/acme-webroot" \
         "${prefix}/var/lib/vps-sub-meter/certificates"
  rmdir "${prefix}/var/lib/vps-sub-meter" 2>/dev/null || true
else
  rm -rf "${prefix}/etc/vps-sub-meter" "${prefix}/var/lib/vps-sub-meter"
fi

# 8. 清理系统用户与用户组（仅在真实根目录且未保留配置/内核时）
if [[ "$ROOT" == "/" && "$KEEP_CONFIG" -eq 0 && "$KEEP_CORES" -eq 0 ]]; then
  for u in vps-sub-meter xray sing-box; do
    if id -u "$u" >/dev/null 2>&1; then
      if ! command -v pgrep >/dev/null 2>&1 || ! pgrep -u "$u" >/dev/null 2>&1; then
        userdel "$u" 2>/dev/null || true
      fi
    fi
  done
  for g in vps-sub-meter xray sing-box vsm-cert; do
    if getent group "$g" >/dev/null 2>&1; then
      groupdel "$g" 2>/dev/null || true
    fi
  done
fi

echo "vps-sub-meter 卸载完成。"
if [[ "$KEEP_CONFIG" -eq 1 ]]; then
  echo "已按 --keep-config 保留配置与流量账本（/etc/vps-sub-meter、/var/lib/vps-sub-meter/traffic-state.json）。"
fi
if [[ "$KEEP_CORES" -eq 1 ]]; then
  echo "已按 --keep-cores 保留代理内核（/usr/local/bin/{xray,sing-box,cloudflared}）。"
fi
if [[ -d "${prefix}/etc/letsencrypt/live" || -d "${prefix}/root/.acme.sh" ]]; then
  echo "提示: /etc/letsencrypt/live/* 与 /root/.acme.sh 下的证书已保留。"
fi
if [[ -n "$BACKUP_CREATED" ]]; then
  echo "备份文件: ${BACKUP_CREATED}"
fi
}
