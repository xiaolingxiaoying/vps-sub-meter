# sbctl

`sbctl` 是 sing-box 服务端管理工具，用于在 Debian/Ubuntu VPS 上安装和管理 sing-box、启用代理协议并生成私有订阅。

本仓库是 [`singbox-sub-me`](https://github.com/xiaolingxiaoying/singbox-sub-me) 工作区里**服务端部分的独立副本**（拆分溯源见 [`docs/repository-split.md`](docs/repository-split.md)）。终端与桌面客户端（`sbtui`、`sbcli`、`sbgui`、`client-core`）不在这里，它们仍住在 monorepo。

## 功能

- 管理 VLESS Reality、VMess WebSocket、Hysteria 2、TUIC 和 AnyTLS 五种协议。
- 提供 sing-box 完整/精简、Clash/Mihomo（含 1.18 兼容）、URI、Base64 URI、Shadowrocket 订阅。
- 管理 systemd 服务、TLS 证书、流量统计和签名更新，并在更新失败时回滚。
- **VPS 侧可扩展**：分层覆写文件与操作员自定义分流规则列表，不必改源码。
- **命名订阅凭据**：按设备发放链接，泄露时只吊销一台，支持宽限期与定时轮换。
- **观测**：内核版本、进程内存/CPU/重启次数、journal 日志、活跃连接。
- **DNS 档位**：`cn-direct`（默认）与 `privacy`（DoH 直连解析），加 IPv4-only 开关。
- 服务端 `clash_api` 观测端点（仅 127.0.0.1 + 随机密钥，默认关闭）。

## 系统要求

- Debian 12 或 Ubuntu 22.04 及以上版本
- amd64 或 arm64 VPS，使用 systemd
- root 权限
- 推荐准备解析到 VPS 的域名。Direct 模式需要公网 TCP 80/443；协议端口须按 `sbctl node` 输出自行在云防火墙和系统防火墙中放行。

## 安装

在 VPS 上下载经过签名校验的安装脚本并运行：

```bash
curl -fL --retry 3 -o /tmp/sbctl-install.sh \
  https://github.com/xiaolingxiaoying/singbox-sub-me/releases/latest/download/install.sh
test -s /tmp/sbctl-install.sh && sudo bash /tmp/sbctl-install.sh
```

如果已登录为 `root`，可将最后一行改成 `test -s /tmp/sbctl-install.sh && bash /tmp/sbctl-install.sh`。

> 该地址是**签名信任链的一部分**，不是笔误：`sbctl update` 与安装脚本的构建期公钥都钉在 monorepo 的 Release 上。要让本仓库独立发布 Release，必须同时更换清单地址和 `scripts/install.sh` 里的公钥，见 [`docs/release-signing.md`](docs/release-signing.md)。

### 在 `vps-sub-meter` 分支测试当前源码

下面的步骤从 `vps-sub-meter` 仓库的 `codex/vps-override-cli-fixes` 分支源码构建，不经过正式 Release 签名流程。请只在全新、可丢弃的 Debian 12 / Ubuntu 22.04+ 测试 VPS 上执行；`sbctl install --guided` 会安装 sing-box、写入 `/etc/sbctl` 配置并创建 systemd 服务。不要把这条测试流程用于承载业务的 VPS，也不要在测试期间执行 `sbctl update`（更新命令面向 monorepo 的正式签名 Release）。

以有 `sudo` 权限的普通用户执行：

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates curl git build-essential cmake perl pkg-config

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
rustup toolchain install stable
rustup default stable
rustc --version  # Rust 1.85 或更新版本

SBCTL_TEST_DIR="$(mktemp -d /tmp/sbctl-branch-test.XXXXXX)"
git clone --depth 1 --branch codex/vps-override-cli-fixes \
  https://github.com/xiaolingxiaoying/vps-sub-meter.git "$SBCTL_TEST_DIR/repo"
cd "$SBCTL_TEST_DIR/repo/sbctl"
cargo build --release --locked -p sbctl --no-default-features
sudo install -o root -g root -m 0755 target/release/sbctl /usr/local/bin/sbctl
sudo /usr/local/bin/sbctl install </dev/null && \
  sudo /usr/local/bin/sbctl install --guided
```

`install </dev/null` 是只读预检；只有预检通过才会继续引导安装。正式 Release 安装方式仍见上方命令；仓库内 `scripts/install.sh` 是构建模板，不能直接下载执行。

安装向导会询问订阅模式、域名或 IP、出口网卡、启用的协议与客户端模板。Direct 模式适用于域名已解析到 VPS 的情况；已有 Nginx/Caddy 时选择 External proxy；没有域名时可选择安全性较低的 IP fallback。

非交互安装常用开关：

```bash
sbctl install --guided                 # 先问全部问题，再落地服务与状态
sbctl install --mode direct --subscription-host sub.example.com
sbctl install --manage-firewall        # 事务内执行 ufw allow（只加不删，默认关）
sbctl install --ipv4-only              # 内核出站解析钉在 IPv4
sbctl install --disable-protocol tuic  # 可重复，端口用 --vless-port 等指定
```

安装程序默认不会改动防火墙，也不会接管现有 sing-box 或反向代理；它会打印需要你自己执行的 `ufw allow` 命令。Direct + 域名模式还会在健康检查通过后尝试签发订阅证书，失败时降级为待办提示而不是回滚整个安装，`sbctl status` 会标注"证书未签发"。

更多安装细节见 [`docs/installation.md`](docs/installation.md)。

## 常用命令

```bash
sbctl menu                    # 交互式管理菜单（也可运行 ly）
sbctl status [--json]         # 部署状态：协议、订阅、内核版本、内存、重启次数
sbctl node [--links --qr]     # 节点与协议端口（分享链接只打到终端）
sbctl sub [--format FORMAT]   # 订阅地址
sbctl qr [FORMAT|--all]       # 终端二维码
sbctl logs [-u sing-box|sbctl|all] [-n N] [-f]   # 受管服务的 journal
sbctl restart                 # 校验活动配置并重启两个受管服务
sbctl sing-box version        # 内核完整版本（含 patch）与官方最新稳定版对比
sbctl sing-box status         # 进程事实：状态、PID、内存、CPU、运行时长
sbctl sing-box api enable     # 打开 127.0.0.1 观测端点后，可用下面两条
sbctl sing-box connections    # 活跃连接表
sbctl credential list         # 命名凭据（密钥打码）
sbctl credential add --name phone
sbctl credential revoke phone --grace 30m
sbctl credential rotate [--name phone] [--grace 30m] [--install-timer]
sbctl rule add direct DOMAIN-SUFFIX,example.com
sbctl rule list [direct|proxy|reject|fakeip-filter]
sbctl rule-set add <名称> --url https://…/foo.srs --outbound proxy
sbctl rule-set import-list --url https://…/direct.list --into direct
sbctl rule-set interval 12h   # 订阅端重新下载远程规则集的间隔
sbctl config show                # 部署摘要（不含凭据）
sbctl config wizard              # 交互式改配置
sbctl config override edit <sing-box|clash|server> [--layer <file>]
sbctl traffic show            # 账期流量
sbctl update [--check]        # 校验并应用签名发布清单
sbctl uninstall [--purge]     # 卸载并保留备份
```

## 配置扩展

服务端配置不必硬编码，四类落点都在 `/etc/sbctl/`：

- **分层覆写**：`overrides/sing-box.d/*.json`、`clash.d/*.yaml` 按文件名顺序叠在单文件之后；`sing-box-server.json` + `sing-box-server.d/` 是**本台 VPS 实际运行的配置**的扩展缝。覆写文件顶层可写 `rules_mode = "append" | "prepend" | "replace"`（合并前剥离）。服务端覆写碰到入站凭据字段会被拒绝，合并结果一律过真核 `sing-box check` 再进原子事务。
- **自定义规则列表**：`rules/{direct,proxy,reject,fakeip-filter}.list`，行格式 `DOMAIN-SUFFIX,example.com` / `IP-CIDR,10.0.0.0/8`，`#` 注释，与 qichiyuhub/rule 的 `.list` 一致。
- **远程规则集注册表**：`config.toml` 的 `[[client_extra_rule_sets]]`，只接受 https 的 `.srs`/`.mrs`（由订阅端自己下载）；纯文本列表走 `sbctl rule-set import-list`。
- **编辑器**：`sbctl config override edit <sing-box|clash|server>` 按 `$VISUAL → $EDITOR → vim → nano → vi` 探测，没有 vim 时自动降到 nano（Windows 落到 notepad）；`--layer <文件名>` 可直接编辑 `.d/` 中的分层文件。

策略组名已去掉前置图标（`节点选择`、`自动选择`、`流媒体`、`Telegram` 等），对客户端透明。详见 [`docs/subscription-guide.md`](docs/subscription-guide.md) 与 ADR-0029／0030／0031（[`docs/adr/`](docs/adr/)）。

### 覆写 CLI

`sbctl config override show` 会列出三个目标的基础文件和有效 drop-in 层。`edit` 支持 `sing-box`、`clash`、`server`；使用 `--layer 10-dns.json` 或 `--layer 20-routing.yaml` 可编辑对应 `.d/` 文件，文件名仅可含 ASCII 字母数字、点、下划线和连字符，且扩展名必须匹配目标格式。

`validate` 会解析全部覆写；部署已初始化且能找到 sing-box 内核时，同时检查合并后的服务端配置和客户端 full profile。未初始化部署时只做覆写结构检查；缺失配置与损坏配置会分别处理，损坏配置返回失败。

`clear` 不带目标时清除 sing-box 与 Clash 客户端覆写（包括基础文件和 drop-in 层）；`clear server` 只清除服务端目标，`clear all` 清除全部目标。清理后会重新生成并校验工件，若生成失败会恢复被清理的文件。

## 从源码构建

```bash
cargo build --release --locked -p sbctl --no-default-features
```

生成的程序为 `target/release/sbctl`。生产 Release 使用 GitHub Actions 构建和签名；不要直接运行仓库里的 `scripts/install.sh`，它没有生产公钥。开发和验收说明见 [`docs/release-signing.md`](docs/release-signing.md) 与 [`tests/acceptance/README.md`](tests/acceptance/README.md)。

## 验证

| 门控 | 命令 |
|---|---|
| L2 Linux（fmt / clippy / 全量测试） | `MSYS_NO_PATHCONV=1 wsl -d Ubuntu-22.04 -- bash scripts/dev/wsl-gate.sh` |
| L2 真核矩阵（sing-box 1.10–1.14 + mihomo） | 先 `scripts/dev/fetch-sing-box-cores.sh`，再 `scripts/dev/wsl-real-cores.sh` |
| L3 systemd 验收 | `SBCTL_ARTIFACT=… SBCTL_TEST_ARTIFACT=… SBCTUI_ARTIFACT=… SBCLI_ARTIFACT=… sh tests/acceptance/run.sh` |

L3 的两条客户端断言（`verify-client.sh`、`verify-sbcli.sh`）需要 monorepo 构建的 `sbtui`/`sbcli` 二进制——本仓库没有这两个 crate，`scripts/dev/build-acceptance-artifacts.sh` 会明确打印它跳过了客户端那一腿。`verify-bootstrap.sh`/`verify.sh`/`verify-real.sh` 三条服务端断言不受影响。

集成分支的 CI ([../.github/workflows/sbctl-branch-ci.yml](../.github/workflows/sbctl-branch-ci.yml)) 从仓库根目录对本目录运行格式检查、安装脚本测试、clippy、Rust 测试和 release 构建。

## 许可证

MIT OR Apache-2.0，详见 [`Cargo.toml`](Cargo.toml)。
