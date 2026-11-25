#!/usr/bin/env bash
set -euo pipefail

# Kernel network tuning (BBR + 16 MiB UDP buffers + disable MPTCP) and
# isolated nftables table (`inet vps_sub_meter_hy2`) for Hysteria2 port hopping.

TABLE="vps_sub_meter_hy2"
CMD="${1:-}"
shift || true

case "$CMD" in
  tune-sysctl)
    cat >/etc/sysctl.d/99-vps-sub-meter.conf <<'EOF'
net.core.default_qdisc = fq
net.ipv4.tcp_congestion_control = bbr
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.core.rmem_default = 1048576
net.core.wmem_default = 1048576
net.ipv4.udp_rmem_min = 16384
net.ipv4.udp_wmem_min = 16384
net.mptcp.enabled = 0
EOF
    sysctl --system >/dev/null
    ;;
  apply-hy2)
    iface=${1:?usage: net.sh apply-hy2 <iface> <start-end> <target_port>}
    range=${2:?usage: net.sh apply-hy2 <iface> <start-end> <target_port>}
    target=${3:?usage: net.sh apply-hy2 <iface> <start-end> <target_port>}
    [[ "$iface" =~ ^[a-zA-Z0-9_.-]+$ ]] || { echo "无效网卡名" >&2; exit 2; }
    [[ "$range" =~ ^[0-9]+-[0-9]+$ ]] || { echo "无效端口范围" >&2; exit 2; }
    [[ "$target" =~ ^[0-9]+$ ]] || { echo "无效目标端口" >&2; exit 2; }
    nft delete table inet "$TABLE" 2>/dev/null || true
    nft -f - <<EOF
table inet ${TABLE} {
  chain prerouting {
    type nat hook prerouting priority dstnat; policy accept;
    iifname "${iface}" fib daddr type local udp dport ${range} counter redirect to :${target} comment "vps-sub-meter Hy2 port hopping"
  }
}
EOF
    ;;
  cleanup-hy2)
    nft delete table inet "$TABLE" 2>/dev/null || true
    ;;
  *)
    echo "用法: $0 {tune-sysctl|apply-hy2|cleanup-hy2}" >&2
    exit 2
    ;;
esac
