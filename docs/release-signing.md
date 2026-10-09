# 生产发布签名配置与旧密钥迁移

仓库中的 `scripts/dev-signing-key.hex` 是公开测试数据。旧安装器信任它对应的公钥，无法证明发布者身份。新代码的普通构建不再信任该密钥；未配置生产公钥时，manifest 安装和更新会明确失败，不能回退到开发密钥。

## 配置一次新的生产密钥

1. 在可信维护设备上运行 `cargo run -p sbctl -- release keygen --output <仓库外的私有目录>`。命令输出公钥，私钥写入该目录；不要把私钥提交到 Git、聊天、日志或发布工件。已有密钥可以继续使用，但更换公钥会使旧二进制无法验证新 manifest。
2. 在 GitHub 仓库 Actions Variables 中设置 `SBCTL_RELEASE_PUBLIC_KEY_HEX`，值为命令输出的 64 位十六进制公钥。
3. 创建 GitHub Environment `release`，在其 Secrets 中设置 `SBCTL_SIGNING_SEED`，值为私钥文件中的 32 字节十六进制 seed。发布工作流只从此 Environment 读取 seed；不要将它设置为公开变量或写入仓库。
4. 推送与 `Cargo.toml` 版本匹配的 `sbctl-v*` 标签（当前版本为 `sbctl-v0.0.4`）。[`.github/workflows/release.yml`](../.github/workflows/release.yml) 在 amd64 和 arm64 runner 构建普通 `sbctl`，再由 package job 用 Environment 私钥签名并由对应二进制验签。公私钥不匹配、缺少密钥或仍使用公开开发密钥时，流程必须在上传发布工件前失败。

## 发布流水线的四个阶段

| Job | 做什么 | 失败模式 |
|---|---|---|
| `validate` | 标签必须等于 `sbctl-v<Cargo.toml 版本>` 且指向 `HEAD`；仓库变量 `SBCTL_RELEASE_PUBLIC_KEY_HEX` 必须存在且不是公开开发密钥 | 标签/版本/锚点任一不符即中止，不构建 |
| `build` | 带锚点编译，跑 `scripts/dev/release-trust-anchor-check.sh` 与 `--test release_trust` | 二进制信任开发密钥、或没配锚点，拒绝上传 |
| `runtime` | 按 [`scripts/release-runtime-pins.txt`](../scripts/release-runtime-pins.txt) 下载随包 sing-box，校验钉住的 sha256，再与 GitHub API 的 `digest` 交叉核对；只解出 `sing-box-<版本>-linux-<arch>/sing-box` 一个成员 | 摘要缺失/不符、pin 过期、成员名危险，拒绝签名 |
| `package` | 渲染 `install.sh`，逐架构 `generate-manifest.sh` 签名并用发布二进制回验，生成 `SHA256SUMS`，以**草稿**发布、上传全部资产、再 `--draft=false` | 半途失败只留下可删除重跑的草稿；已发布的 tag 拒绝覆盖 |

`scripts/generate-manifest.sh` 从同一个 pin 文件读取版本与兼容区间：传入的 sing-box 版本必须等于 `sing_box_version`，否则拒绝签名，避免清单声明一个没有随包发布的运行时。

`scripts/prepare-installer.py` 将同一个生产公钥写入发布工件 `install.sh`。仓库里的 `scripts/install.sh` 是未配置的模板，直接执行会失败。README 的安装入口指向 `https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh`。发布会附上 `sbctl-linux-amd64`、`sbctl-linux-arm64`、两个 sing-box 运行时、两个签名 manifest、`install.sh` 与 `SHA256SUMS`。

> 兼容区间写在该 pin 文件里（当前 `1.10.0:1.14.99`），与 CI `sing-box-profiles` job 覆盖的版本带一致；升级随包内核时同时更新版本、逐架构摘要与区间。

### 发布门禁

```bash
bash scripts/dev/release-trust-anchor-check.sh target/release/sbctl
```

门禁是行为化的：用公开的开发种子签一个合法 manifest，要求发布二进制**拒绝**它，且拒绝原因不是未配置锚点，并另验一个 schema 2 的 manifest 仍被拒绝。它同时挡住两类事故：误用 `--features test-signing` 构建，以及忘记注入 `SBCTL_RELEASE_PUBLIC_KEY_HEX`。

> 本仓库 `release.yml` 与 `ci.yml` 的第三方 action 目前仍是浮动 tag（`@v4`/`@stable`），属于已知缺口（审查报告 P2-18）。

已有旧版本不会自动获得新的信任根；旧 updater 无法验证新 manifest。迁移时需要通过可信渠道取得带新公钥的 `sbctl` 二进制，独立核对发布来源及公钥后手动替换，再使用新 updater。不要把旧开发公钥作为兼容备用公钥保留。仅删除开发私钥文件或重写仓库历史都不能修复已经发布的旧二进制。

生产构建在 GitHub Actions 编译时提供 `SBCTL_RELEASE_PUBLIC_KEY_HEX`。这是编译期信任根；已构建的二进制不能通过修改运行环境改变它。VPS 用户应从 Release 运行安装器，无需本地构建。

## 测试与验收隔离

```bash
cargo test --workspace --features sbctl/test-signing
cargo test -p sbctl --no-default-features --test release_trust
```

`test-signing` 是显式测试开关，供 CLI 和签名回滚 fixture 使用。启用该特性的二进制不得发布。普通 `cargo test` 不运行需要该特性的 `cli` 目标；CI 显式运行以上两条命令，分别验证签名业务流程和生产构建拒绝公开开发签名的边界；发布流水线的 `build` job 另跑行为化门禁（见上）。

systemd 验收分别传入 `SBCTL_ARTIFACT`（生产构建）和 `SBCTL_TEST_ARTIFACT`（独立目录中的测试签名构建）。公开测试签名的回滚场景使用后者；真实服务安装、非 root 启动和卸载使用前者。测试安装器只在验收容器的临时目录注入测试公钥，不会进入发布工件。

`SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT` 在本仓库是**可选**的：本仓库只有服务端 crate，未设置时 `tests/acceptance/run.sh` 会打印 `branch: server-only workspace - skipping the client legs` 并只跑三条服务端断言。

sbctl 服务端 Release 只发布 `sbctl`、它管理的 sing-box 运行时、签名 manifest 和安装脚本；TUI/GUI 不参与服务端发布。正式发布前仍须确认 `release` Environment 已配置上述 seed，并验证 Actions 的签名、安装和 systemd 验收全部通过。
