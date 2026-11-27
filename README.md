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

日常直连优先选 `vless-reality-pqc`。UDP 通畅且线路延迟较高或有丢包时，可以对比 `hysteria2-hop` 的下载吞吐。需要接入 HTTPS 反代或 CDN 时，优先考虑 `vless-xhttp-pqc`。`anytls-tcp` 可作 TCP 备用，常转发游戏或语音 UDP 时可对比 `tuic-bbr` 的延迟和抖动。

新装向导可使用模板默认端口、随机端口，或逐个手动输入端口。随机方案让 `vless-reality-pqc` 优先使用可用的 `443/tcp`，其余节点使用 `20000–29999`。域名模式下 REALITY 可与订阅 HTTPS 通过 SNI 复用 `443/tcp`。已有部署可通过 `ly` 的 `11. 节点端口设置` 修改单个节点、套用模板或重新随机分配。证书管理采用纯 Shell `acme.sh`，新装部署无需 Python 依赖。

REALITY 新装默认 SNI 是 `www.icloud.com`，来自 `/etc/vps-sub-meter/templates/default-config.toml` 的 `[nodes.vless_reality_pqc]` → `decoy_sni`，编辑模板即可调整新装默认值。已有部署应在主配置 `/etc/vps-sub-meter/config.toml` 里编辑同一字段，运行 `vsm apply` 后刷新客户端订阅。

另可选原生集成 **Cloudflare Argo 隧道优选订阅**。`ly` 的 `2. 查看订阅与二维码` 可以先选直连或 Argo，再选客户端格式。`7. 管理 Argo 隧道` 可检查本机与公网订阅。节点和订阅域名的 DNS 需指向该隧道，域名相同时本机生成的回源规则按 URL 路径分流。若在 Cloudflare 控制台配置了回源规则，也应将 `/sub/…` 指向订阅监听端口，并将节点 WebSocket 路径指向对应 WS 入站。

在 `ly` → `7. 管理 Argo 隧道` → `1. 配置并启用` 或 `5. 更新优选 IP` 中，可输入最多 32 个公网 IPv4 地址，用逗号或空格分隔。回车保留当前设置，输入 `清空` 仅使用隧道域名，输入 `文件` 则从 `preferred_ips_file` 读取（默认 `/etc/vps-sub-meter/argo-ips.txt`）。确认后会重新生成订阅。也可以直接编辑 `config.toml` 中 `[argo]` 下的 `preferred_ips`，或在文件模式下更新 IP 文件，再运行 `vsm apply`。

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

- 交互菜单：运行 `ly` → 选择 `9. 卸载 vps-sub-meter`（菜单同时内置内核更新、`6. 配置 / 切换域名 (HTTPS)` 与 Argo 隧道管理）
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
vsm panel              # 按需打开仅供 SSH 转发访问的网页管理面板
vsm sub --qr           # 输出订阅链接与终端二维码
vsm apply              # 校验外部配置/模板，检查官方内核后平滑生效
vsm traffic checkpoint # 立即刷新流量账本到磁盘
```

运行 `ly`，选择 `10. 流量账期与校准`，可修改月流量上限、每月重置日、时间和时区，也可校准已用上传和下载或立即保存流量账本。修改重置日程时保留本期已用流量。

`ly` 的 `12. 证书管理` 显示当前证书的签发者和有效期，也可续期、同步或申请域名证书。IP 模式可补齐自签证书并重新发布订阅。

域名模式使用常规 TLS 证书验证。IP 模式在 Mihomo 配置中固定服务器证书的 SHA-256 指纹，在 sing-box 配置中固定证书公钥的 SHA-256 指纹。XHTTP 分享链接使用 `pcs`，Hysteria2 链接使用 `pinSHA256`，需要客户端支持对应字段。IP 模式的 AnyTLS、TUIC 分享链接仍使用兼容参数跳过证书验证。对证书验证有要求时，优先使用 Mihomo 或 sing-box 的完整配置。

IP 模式的直连订阅通过 HTTP 发布，其中包含连接凭据和指纹。请通过 SSH 等可信渠道获取配置并导入，或绑定域名启用 HTTPS。手动更换 IP 模式证书后，运行 `vsm apply` 重新生成配置，再用 `cert.sh reload www.bing.com` 让内核加载证书，并通过可信渠道刷新客户端。

节点和 Argo 在 `config.toml` 中编辑，保存后执行 `vsm apply`。Argo 隧道部署用 `argo.sh setup`。组件更新用 `cores.sh update`，证书续期用 `cert.sh renew`，内核参数调优用 `net.sh tune-sysctl`。这些脚本安装在 `/usr/local/lib/vps-sub-meter/`。

## 网页管理面板

在你自己的电脑上运行以下命令，把 `admin@vps.example.com` 换成实际的 SSH 用户和主机：

```bash
ssh -t -o ExitOnForwardFailure=yes -L 127.0.0.1:8787:127.0.0.1:8787 admin@vps.example.com 'sudo /usr/local/bin/vps-sub-meter panel'
```

在浏览器打开 `http://127.0.0.1:8787/`，输入 SSH 终端显示的会话密钥。密钥仅在此次面板进程运行期间有效。保持 SSH 连接，等待运维操作完成后按 Ctrl+C 关闭面板。端口被占用时，给面板加 `--port 其他端口`，并同步修改 SSH 转发两端的端口和浏览器地址。

面板内置于 `vps-sub-meter` 二进制，只监听 VPS 的 `127.0.0.1`，无需额外安装 Web 服务、开公网端口或启动常驻进程。它复用项目已有的配置、密钥、流量账本及 Bash 运维脚本。

- **概览、部署配置、节点端口** 展示流量和已发布批次，支持初始化、完整配置 JSON、模板或随机端口，以及预览、仅保存或应用配置。
- **流量、订阅与设备凭据** 支持账期设置、流量校准和保存、各设备的直连和 Argo 订阅链接及二维码、凭据新增和吊销、默认凭据与节点密钥轮换。
- **域名与 HTTPS、Argo、证书** 支持域名绑定、证书状态和签发、同步或续期，以及 Argo 启停、优选 IP、回源刷新和连接检查。
- **系统维护** 支持重新应用配置、更新代理内核、网络参数调优、升级检查、安装官方发行包，以及保留配置或彻底卸载。

手动修改本期用量时，在「流量账期」的「流量校准」中填写上传和下载，点击「保存已用流量」并确认。「账本维护」中的「记录当前统计」用于立即记录服务器当前统计的用量。

重置时区和显示时区可以分别选择美西（洛杉矶）、美东（纽约）等常用时区，也可以填写自定义 IANA 时区名称。

## 脚本接口

`vsm status --json` 返回字节数、重置时间戳和当前批次。管理员可用 `vsm sub --json` 读取 `direct` 及可选的 `argo` 订阅链接。这些链接含私有凭据。用 `--token-name 名称` 选择指定设备，用 `--qr-format clash|sing-box|shadowrocket` 选择终端二维码格式。Argo Shadowrocket 二维码可用 `vsm sub --argo --qr-format shadowrocket` 输出。

`vsm token list --json` 只输出脱敏列表。`vsm token add 名称` 新增凭据，`vsm token rotate` 轮换默认凭据，`vsm token revoke 名称` 吊销指定凭据。`vsm keys rotate --yes` 轮换节点密钥并应用，客户端随后需要刷新订阅。

`vsm config show` 输出当前配置 JSON，`--defaults` 读取新装模板。`vsm config prepare` 生成并校验候选配置，可用 `--port anytls_tcp=24443` 指定单个节点端口。将 `.config` 字段交给 `vsm config apply` 可直接校验并应用，失败时恢复原配置。`vsm config write` 仅保存，覆盖已有主配置需要显式指定 `--replace`。

```bash
vsm config prepare --domain sub.example.com --public-ip 203.0.113.20 \
  --interface eth0 --monthly-gib 500 --reset-day 1 --ports random > candidate.json
jq -c '.config' candidate.json | vsm config write
```
