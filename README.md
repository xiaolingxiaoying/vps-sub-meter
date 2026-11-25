# vps-sub-meter

`vps-sub-meter`（快捷命令 `vsm` / `ly`）是面向 Debian / Ubuntu VPS 的**双内核（Xray + sing-box）后量子 PQC 代理与私密订阅控制面**。

核心设计原则：**最小 Rust 常驻核心 + 人类可读外部配置文件 + 模块化 Bash 运维脚本**。

## 架构与 5 主力节点

| 序号 | 节点名称 | 内核 | 默认端口 | 协议栈特性 |
| --- | --- | --- | --- | --- |
| 1 | `vless-reality-pqc` | Xray `26.11.9` | `443/tcp`（SNI 分流至回环 `1443`） | VLESS + `xtls-rprx-vision` + REALITY (`ML-DSA-65`) + VLESS Encryption (`ML-KEM-768`) |
| 2 | `vless-xhttp-pqc` | Xray `26.11.9` | `2053/tcp` | VLESS + XHTTP (`mode: auto`) + TLS 1.3 (`h2`) + VLESS Encryption (`ML-KEM-768`) |
| 3 | `anytls-tcp` | sing-box `1.14+` | `2083/tcp` | AnyTLS + TLS 1.3 + 8 段 `padding_scheme` |
| 4 | `hysteria2-hop` | sing-box `1.14+` | `34443/udp`（跳跃 `30000-35000`） | Hysteria2 + Salamander 混淆 + `nftables` 防 QoS 端口跳跃 |
| 5 | `tuic-bbr` | sing-box `1.14+` | `2096/udp` | TUIC v5 + BBR + 0-RTT + Native UDP |

另可选原生集成 **Cloudflare Argo 隧道优选订阅**（直接读取 `/etc/vps-sub-meter/argo-ips.txt` 动态展开优选 IP，无需额外 Python 服务）。

## 快速安装与全版本平滑升级

### 1. 全新安装（Debian / Ubuntu）

```bash
curl -fsSL https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh | sudo bash
```

### 2. 从任意历史版本一键平滑升级（保留节点、凭据、域名、证书与流量账本）

`scripts/upgrade.sh` 统一兼容以下全部历史版本，并在操作前自动备份至 `/root/pre-vsm-upgrade-*.tar.gz`：

- **v1/v2 早期脚本版**（`/etc/sub-srv/config.conf` + `/var/lib/subsrv/tx_state.json` + Caddy / `sing-box-yg`）
- **`sbctl` 系列**（`sbctl-v0.0.1` ~ `sbctl-v0.0.28`，`/etc/sbctl/config.toml` + `/var/lib/sbctl/state.json`，含混合 PQC / Argo 部署）
- **`vps-sub-meter` v2.0.0+**（`/etc/vps-sub-meter/config.toml` 就地升级）

```bash
# 先检查当前环境与迁移计划（只读，不做任何改动）
curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/dev/scripts/upgrade.sh | sudo bash -s -- --check

# 执行平滑升级（自动优先使用已签名 Release；无预编译产物时自动从源码编译）
curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/dev/scripts/upgrade.sh | sudo bash
```

详见 [升级指南](docs/upgrade.md)。

## 目录布局

- 主程序与快捷命令：`/usr/local/bin/vps-sub-meter`（软链接 `vsm`、`ly`）
- 配置文件：`/etc/vps-sub-meter/config.toml`（修改后运行 `vsm apply` 校验并平滑生效）
- 密钥仓库：`/etc/vps-sub-meter/secrets.json`（`0600 root:root`，绝不进入订阅目录）
- 外置客户端模板：`/etc/vps-sub-meter/templates/{mihomo.yaml, sing-box.json}`
- 运维脚本库：`/usr/local/lib/vps-sub-meter/{cores.sh, cert.sh, net.sh, argo.sh, upgrade.sh, migrate_legacy.py, migrate-sbctl.sh}`
- 订阅批次与流量账本：`/var/lib/vps-sub-meter/{current.json, serve-auth.json, traffic-state.json, generations/, certificates/}`

## 精简订阅端点

所有订阅端点均使用路径凭据校验（拒绝任何 `?query` 参数，错误 Token 统一返回 `404`）：

- `/sub/<token>/clash.yaml` — Mihomo / Clash Party 完整配置（含 5 节点 + `TCP优选` / `自动选择`，内置 TUN/DNS 国内直连抗阻塞优化与 Fake-IP 防漏规则）
- `/sub/<token>/sing-box.json` — sing-box `1.14+` 完整配置（自动过滤 Xray 专属 PQC 节点，保留 AnyTLS + Hy2 + TUIC）
- `/sub/<token>/shadowrocket.txt` — Shadowrocket Base64 订阅
- `/sub/<token>/uri.txt` — 明文分享 URI
- `/sub/<token>/index` — 暗色中文流量看板与内联 SVG 二维码页

## 常用命令

```sh
ly                     # 打开单屏 8 项交互菜单（任一步输入 0 立即返回）
vsm status             # 查看当前双核状态与实时流量
vsm sub --qr           # 输出订阅链接与终端二维码
vsm apply              # 校验外部配置/模板，检查官方内核后平滑生效
vsm traffic checkpoint # 立即刷新流量账本到磁盘
```
