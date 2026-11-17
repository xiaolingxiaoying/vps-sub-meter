# 服务端构建与验证流程

本仓库只包含 `sbctl` 服务端和 `crates/json-merge`；版本以根目录 `Cargo.toml` 为准。桌面、终端客户端在独立仓库，不属于这里的构建目标。

## Windows 与 WSL 回归

Windows 仓库根目录：

```powershell
cargo fmt --all -- --check
python -m unittest discover -s scripts -p 'test_*.py'
node tests/index-copy.mjs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --features test-signing
cargo test --no-default-features --test release_trust
```

`test-signing` 启用 CLI 集成测试，不能用于发布。生产签名信任测试必须单独关闭该特性，确认普通构建拒绝开发签名。

Windows 宿主调用 WSL Ubuntu 22.04：

```powershell
wsl -d Ubuntu-22.04 -- bash /mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/scripts/dev/wsl-gate.sh
```

脚本加载 Rust 环境，将源码复制到 `~/ws/vps-sub-meter`，在 ext4 上执行格式、Clippy 和工作区测试；默认构建目录为 `~/ws/target-vps`。其他机器可通过 `SRC`、`DST`、`CARGO_TARGET_DIR` 覆盖路径。脚本不负责 Python、Node 或生产签名信任的单独检查，上述命令仍须执行。

## 固定真实内核

普通 `cargo test` 会跳过需要外部内核的测试；它们通过不代表完整兼容性矩阵已经执行。

准备固定版本后运行 `scripts/dev/verify-managed.sh`，脚本所需的 Xray 归档、sing-box/Mihomo 路径见脚本和 [CI 配置](../.github/workflows/ci.yml)。仅执行托管矩阵时：

```bash
SBCTL_XRAY_BIN=/absolute/path/to/xray \
SING_BOX_BIN_1_10=/absolute/path/to/sing-box-1.10.7 \
SING_BOX_BIN_1_11=/absolute/path/to/sing-box-1.11.15 \
SING_BOX_BIN_1_12=/absolute/path/to/sing-box-1.12.25 \
SING_BOX_BIN_1_13=/absolute/path/to/sing-box-1.13.21 \
SING_BOX_BIN_1_14=/absolute/path/to/sing-box-1.14.1 \
MIHOMO_BIN=/absolute/path/to/mihomo \
  cargo test --test managed_deployment -- --ignored --nocapture
```

完整版本档、Xray 与协议转发检查使用 CI 中的固定版本和命令。1.14.4 的空缓存规则集初始化测试还需要对应内核与外网访问；`check` 成功不能证明远程规则集下载或实际转发成功。

`verify-managed.sh` 还执行 `tests/probe.rs`，用真实 sing-box 验证 `NO_PROXY=*` 无法绕过失效节点。两台授权 VPS 的链接保留与就地部署结果见 [2026-11-16 验收记录](two-vps-verification-2026-11-16.md)。

## systemd 发行版验收

先在 Linux/WSL 构建独立生产与测试工件：

```bash
cargo build --release --locked --no-default-features
cargo build --release --locked --features test-signing --target-dir target-fixtures
```

在已启用 Docker 的环境，从本仓库根目录执行：

```bash
SBCTL_ARTIFACT="$PWD/target/release/sbctl" \
SBCTL_TEST_ARTIFACT="$PWD/target-fixtures/release/sbctl" \
  sh tests/acceptance/run.sh
```

Windows Docker Desktop 场景须把 Linux 工件放到 Docker 能访问的宿主路径，再提供其绝对路径。不要传入 Windows `.exe`。服务端仓库不提供客户端工件，验收脚本会跳过依赖独立客户端仓库的步骤。

该入口在隔离容器中检查 Debian 12、Ubuntu 22.04/24.04 的 systemd 安装、服务权限、订阅模式、更新与恢复。执行条件、特权容器边界见 [验收说明](../tests/acceptance/README.md)。禁止直接在日常 VPS 上运行破坏性验收脚本。

WSL 单测和固定内核检查不能替代真实 systemd 验收；开机中断恢复还须验证服务自启动状态。已执行证据与待完成门禁见 [托管部署验收记录](managed-verification-2026-11-14.md)。本地未注入正式签名公钥的构建不得作为正式发布工件；发布流程见 [签名发布说明](release-signing.md)。
