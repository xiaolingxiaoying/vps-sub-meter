# vps-sub-meter

`vps-sub-meter`（快捷命令 `vsm` / `ly`）是面向 Debian / Ubuntu VPS 的**双内核（Xray + sing-box）后量子 PQC 代理与私密订阅控制面**。

核心设计原则：**最小 Rust 常驻核心 + 人类可读外部配置文件 + 模块化 Bash 运维脚本**。

## 架构与 5 主力节点

| 序号 | 节点名称 | 内核 | 默认端口 | 协议栈特性 |
| --- | --- | --- | --- | --- |
| 1 | `vless-reality-pqc` | Xray `26.11.9` | 优先 `443/tcp`（被占则随机） | VLESS + `xtls-rprx-vision` + REALITY (`ML-DSA-65`) + VLESS Encryption (`ML-KEM-768`) |
| 2 | `vless-xhttp-pqc` | Xray `26.11.9` | 随机 TCP | VLESS + XHTTP (`mode: auto`) + TLS 1.3 (`h2`) + VLESS Encryption (`ML-KEM-768`) |
| 3 | `anytls-tcp` | sing-box `1.14+` | 随机 TCP | AnyTLS + TLS 1.3 + 8 段 `padding_scheme` |
| 4 | `hysteria2-hop` | sing-box `1.14+` | 随机 UDP（默认跳跃范围 `30000-35000`） | Hysteria2 + Salamander 混淆 + `nftables` 防 QoS 端口跳跃 |
| 5 | `tuic-bbr` | sing-box `1.14+` | 随机 UDP | TUIC v5 + BBR + 0-RTT + Native UDP |

全新安装时，端口方案默认采用「`vless-reality-pqc` 优先使用 `443/tcp`（域名模式下与订阅 HTTPS 通过 SNI 复用 `443/tcp`，若本机 `443` 已被占用则自动回退到随机高位端口），其余 4 个节点在 `20000–29999` 内随机分配」，确认后写入 `/etc/vps-sub-meter/config.toml`。后续应用和升级会沿用已保存的端口，重新运行向导时默认也会保留现有端口。证书管理采用纯 Shell `acme.sh`，新装部署无需 Python 依赖。

REALITY 新装默认 SNI 是 `www.icloud.com`，来自 `/etc/vps-sub-meter/templates/default-config.toml` 的 `[nodes.vless_reality_pqc]` → `decoy_sni`，编辑模板即可调整新装默认值。已有部署应在主配置 `/etc/vps-sub-meter/config.toml` 里编辑同一字段，运行 `vsm apply` 后刷新客户端订阅。

另可选原生集成 **Cloudflare Argo 隧道优选订阅**（在 `vsm apply` 时读取 `/etc/vps-sub-meter/argo-ips.txt` 预编译并展开优选 IP，更新优选 IP 后执行一次 `vsm apply` 即可生效）。

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

- 主程序：`/usr/local/bin/vps-sub-meter`（快捷命令 `vsm`）。`ly` 指向外置 Bash 菜单 `/usr/local/lib/vps-sub-meter/ly.sh`。
- 主配置：`/etc/vps-sub-meter/config.toml`（修改后运行 `vsm apply` 校验并生效）。主配置完整时可独立读取。旧配置缺少必填字段时，用默认模板补齐，主配置中已填写的值优先。
- 默认配置模板：仓库的 `templates/default-config.toml` 安装到 `/etc/vps-sub-meter/templates/default-config.toml`。新装向导从这里读取默认值，确认后保存到主配置。升级默认保留已安装模板。
- 密钥仓库：`/etc/vps-sub-meter/secrets.json`（`0600 root:root`，绝不进入订阅目录）
- 外置客户端模板：`/etc/vps-sub-meter/templates/{mihomo.yaml, sing-box.json}`
- 运维脚本库：`/usr/local/lib/vps-sub-meter/{cores.sh, cert.sh, net.sh, argo.sh, upgrade.sh, uninstall.sh, migrate_legacy.py, migrate-sbctl.sh}`
- 订阅批次与流量账本：`/var/lib/vps-sub-meter/{current.json, serve-auth.json, traffic-state.json, generations/, certificates/}`

## 精简订阅端点

所有订阅端点均使用路径凭据校验（拒绝任何 `?query` 参数，错误 Token 统一返回 `404`）：

- `/sub/<token>/clash.yaml` — Mihomo / Clash Party 完整配置（含 5 节点 + `TCP优选` / `自动选择`，内置 TUN/DNS 国内直连抗阻塞优化与 Fake-IP 防漏规则）
- `/sub/<token>/sing-box.json` — sing-box `1.14+` 完整配置（自动过滤 Xray 专属 PQC 节点，保留 AnyTLS + Hy2 + TUIC）
- `/sub/<token>/shadowrocket.txt` — Shadowrocket Base64 订阅
- `/sub/<token>/uri.txt` — 明文分享 URI
- `/sub/<token>/index` — 暗色中文流量看板与内联 SVG 二维码页

## 卸载

可通过交互菜单或独立卸载脚本清理（默认保留 `/etc/letsencrypt/live/*` 与 `/root/.acme.sh` 证书，并在彻底卸载前自动备份配置与流量账本至 `/root/vsm-uninstall-backup-*.tar.gz`）：

- 交互菜单：运行 `ly` → 选择 `8. 卸载 vps-sub-meter`（菜单同时内置内核更新与 Argo 隧道管理）
- 本地卸载：
  ```bash
  sudo bash /usr/local/lib/vps-sub-meter/uninstall.sh
  ```
- 在线卸载：
  ```bash
  curl -fsSL https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/uninstall.sh | sudo bash
  ```

常用参数：`--keep-config`（保留配置与流量账本）、`--keep-cores`（保留 Xray/sing-box/cloudflared 内核）、`--no-backup`（跳过自动备份）、`-y / --yes`（跳过确认）。

## 常用命令

```sh
ly                     # 初始化、查订阅、管理 Token、轮换节点密钥或应用配置
vsm status             # 查看订阅批次与实时流量
vsm sub --qr           # 输出订阅链接与终端二维码
vsm apply              # 校验外部配置/模板，检查官方内核后平滑生效
vsm traffic checkpoint # 立即刷新流量账本到磁盘
```

节点、Argo 和流量账期在 `config.toml` 中编辑，保存后执行 `vsm apply`。Argo 隧道部署用 `argo.sh setup`。组件更新用 `cores.sh update`，证书续期用 `cert.sh renew`，内核参数调优用 `net.sh tune-sysctl`。这些脚本安装在 `/usr/local/lib/vps-sub-meter/`。

## 脚本接口

`vsm status --json` 返回字节数、重置时间戳和当前批次。管理员可用 `vsm sub --json` 读取 `direct` 及可选的 `argo` 订阅链接。这些链接含私有凭据。用 `--token-name 名称` 选择指定设备，用 `--qr-format clash|sing-box|shadowrocket` 选择终端二维码格式。

`vsm token list --json` 只输出脱敏列表。`vsm token add 名称` 新增凭据，`vsm token rotate` 轮换默认凭据，`vsm token revoke 名称` 吊销指定凭据。`vsm keys rotate --yes` 轮换节点密钥并应用，客户端随后需要刷新订阅。

`vsm config show` 输出当前配置 JSON，`--defaults` 读取新装模板。`vsm config prepare` 生成并校验候选配置。提交其 `.config` 字段给 `vsm config write` 可保存所预览的端口和设置。覆盖已有主配置需要显式指定 `--replace`，随后运行 `vsm apply`。

```bash
vsm config prepare --domain sub.example.com --public-ip 203.0.113.20 \
  --interface eth0 --monthly-gib 500 --reset-day 1 --ports random > candidate.json
jq -c '.config' candidate.json | vsm config write
```
