# 01 发布流水线：本仓库自己生产签名 Release

Status: ready-for-agent

## 问题

`README.md` 与 `docs/installation.md` 让用户从
`https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh`
安装，`src/update.rs::manifest_url_for_arch` 也从同一处取签名清单，但
`.github/workflows/` 只有 `ci.yml`：它不注入编译期公钥、不签名、不生成
manifest、不生成 `install.sh`、不创建 Release。整条信任链没有生产者。

## 要做

1. 新增 `.github/workflows/release.yml`：
   - 触发：`push` 标签 `sbctl-v*`，以及 `workflow_dispatch(tag)`；
   - `validate`：标签必须等于 `sbctl-v<Cargo.toml 版本>`，且指向 `HEAD`；仓库变量
     `SBCTL_RELEASE_PUBLIC_KEY_HEX` 必须存在且不是公开开发密钥；
   - `build`（amd64/arm64）：带锚点构建，跑 `scripts/dev/release-trust-anchor-check.sh`
     与 `--test release_trust`，产物 `sbctl-linux-<arch>`；
   - `runtime`（amd64/arm64）：按 `scripts/release-runtime-pins.txt` 下载并校验 sing-box
     归档，再与 GitHub API 的 `digest` 交叉核对，只解出 `sing-box` 成员，产物
     `sing-box-linux-<arch>`；
   - `package`（`environment: release`，`contents: write`）：`prepare-installer.py` 渲染
     `install.sh`，逐架构 `generate-manifest.sh` 签名并立即用发布二进制回验，生成
     `SHA256SUMS`，以草稿发布→上传→`--draft=false`。
2. 新增 `scripts/release-runtime-pins.txt`：版本 + 逐架构 sha256 + 兼容区间。
3. `scripts/generate-manifest.sh` 从该文件读取兼容区间，并拒绝与 pin 不符的版本。
4. 新增 `scripts/dev/release-trust-anchor-check.sh` 作为发布门禁。
5. `docs/release-signing.md` 写清 Variable/Environment/标签约定与失败模式。

## 验收

- 在 fork 上配置 Variable 与 Environment，推一个 `sbctl-v*` 标签：草稿资产包含
  `sbctl-linux-amd64/arm64`、`sing-box-linux-amd64/arm64`、`manifest-amd64/arm64.json`、
  `install.sh`、`SHA256SUMS`；`install.sh` 不含 `@SBCTL_RELEASE_PUBLIC_KEY_PEM@`。
- 用发布出的 `install.sh` 在一台干净容器里装通，并跑 `sbctl update --check`。
- 故意把 `release-runtime-pins.txt` 的 sha256 改错：`runtime` job 必须在下载后失败。
- 用 `test-signing` 构建跑门禁脚本：必须失败。

## 实现

`.github/workflows/release.yml`、`scripts/release-runtime-pins.txt`、
`scripts/generate-manifest.sh`、`scripts/dev/release-trust-anchor-check.sh`、
`scripts/test_prepare_installer.py`（pin 一致性单测）、`docs/release-signing.md`。

## 备注

GitHub 托管 arm64 runner 为 `ubuntu-24.04-arm`；若该 runner 在目标仓库不可用，
退化为 `cross` 构建并在本工单记录偏差。
