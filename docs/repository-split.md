# 仓库拆分说明（vps-sub-meter ← singbox-sub-me）

本仓库是 `singbox-sub-me` 工作区里 **sbctl 服务端**的独立副本，拆分日期 2026-10-07，
来源提交 `2b2a411`（分支 `fix-ui`）。拆分方式是一次**文件级复制 + 全新单提交快照**，
没有使用 `git subtree split`，因此本仓库不含历史；`git log` 从这里重新开始。

## 包含什么

| 路径 | 内容 |
|---|---|
| `src/`、`tests/` | 与源仓库逐字节一致（`diff -r` 验证通过）的服务端源码与测试 |
| `crates/json-merge` | 服务端覆写唯一依赖的合并 crate（客户端 `client-core` 也用同一个库，但那边保留了自己的副本） |
| `Cargo.toml` | 工作区成员缩减为 `crates/json-merge`；删除了 `sbgui` 的 `[patch.crates-io]`（annotate-snippets 补丁只为 GUI 存在） |
| `Cargo.lock` | 由缩减后的清单重新生成：1028 个包 → 234 个包，GUI/TUI 依赖（accesskit、slint、gpui、crossterm 等）全部消失 |
| `scripts/` | `install.sh`、`prepare-installer.py`、`generate-manifest.sh`、`dev-signing-key.hex` 与 `scripts/dev/` 的 WSL/真核/验收脚本 |
| `tests/acceptance/`、`docker-compose.acceptance.yml` | systemd L3 验收套件 |
| `docs/adr/`（32 篇）、`docs/agents/`、服务端文档 | 见下方"文档取舍" |
| `AGENTS.md`、`CLAUDE.md`、`CONTEXT.md` | agent 约定与术语表（`CONTEXT.md` 的标题就是 sbctl，被多篇文档引用） |

数量可核对：`src` 58 文件（含 13 个 `.snap`）、`tests` 25、`crates/json-merge` 2、`scripts` 5 个顶层文件 + `scripts/dev` 5 个、`docs/adr` 32、`docs` 顶层 17（16 篇服务端文档 + 本文）、`docs/research` 4。

## 不包含什么（以及为什么）

- `crates/sbcli`、`crates/sbtui`、`crates/sbgui`、`crates/client-core`：客户端栈。
  拆分前已验证 `src/` 与 `tests/` 对它们**只有注释级引用**（`src/lifecycle.rs:761,775`、
  `src/subscription/render/singbox.rs:132`），没有任何代码或构建依赖，所以删除后服务端自洽。
- `prototypes/`、`packaging/windows/`、GUI/TUI 专用脚本。未复制的 `scripts` 顶层项是
  `package-sbgui-qml.ps1`、`package-sbgui-slint.ps1`、`smoke-sbgui-qml.ps1`、
  `smoke-sbgui-slint.ps1` 与 `sbgui-performance/`、`sbgui-qml/`、`sbgui-shot/`、`winvm/`
  四个目录；`scripts/dev` 少的那一个是 `wsl-signal-exit.sh`（用 pty 证明 sbtui 退出清理的门）。
- `.github/workflows/release.yml`：该工作流的 acceptance/publish 任务串了
  `build-sbtui`、`build-sbgui-qml`、MSI 与共享 daemon 产物，照搬必然失败。
  若本仓库要独立发 Release，需按 `docs/release-signing.md` 重新设计。
- 客户端主题文档（`docs` 顶层 11 项）：`client-core-control-api.md`、
  `client-description.md`、`code-review-2026-09-25.md`、`gpui-platform-acceptance.md`、
  `gui-ui-redesign-recommendations.md`、`legacy-gui-known-issues.md`、
  `project-review-2026-09-24.md`、`project-review-round-3-2026-10-02.md`（后两篇主体是客户端，
  服务端与发布治理结论在已复制的 round-2 里）、`qml-prototype-rewrite-acceptance.md`、
  `qml-ui-layout-acceptance.md`、`sbgui-qml-gui-and-kernel-interface.md`；
  `docs/research/` 少 10 篇（gpui 系列 8 篇 + `ratatui-study-for-sbtui.md` + 两篇客户端综述）；
  根目录的 `DESIGN.md`（QML 视觉系统）与 `PRODUCT.md`（含客户端的产品定位）不复制。
- `.scratch/`（本地工单目录）、`target/`、`dist/`。注意 `AGENTS.md` 与
  `docs/agents/issue-tracker.md` 约定工单落在 `.scratch/` 下，本仓库首次建工单需要自己建目录。

保留的服务端文档里有 5 条指向已删除文件的相对链接（链接扫描：48 条相对链接、5 条断链），
逐条列出以免含糊：

| 出处 | 断链目标 | 该文件在哪 |
|---|---|---|
| `docs/adr/0025-keep-gpui-and-slint-clients.md` | `../research/gpui-current-gui-assessment.md` | monorepo |
| `docs/adr/0025-keep-gpui-and-slint-clients.md` | `../research/gpui-enhancement-strategy.md` | monorepo |
| `docs/adr/0028-client-pure-official-grpc.md` | `../client-core-control-api.md` | monorepo |
| `docs/target-spec-gap-and-verification-plan.md` | `../DESIGN.md` | monorepo |
| `docs/target-spec-gap-and-verification-plan.md` | `../PRODUCT.md` | monorepo |

这些是决策记录原文，**故意不改写**，断链指向的是 `singbox-sub-me` 里的对应文件。


## 拆分后必须知道的三件事

1. **自更新与安装脚本仍指向 monorepo 的 Release。**
   `src/update.rs:72`、`scripts/install.sh:64` 里的清单 URL 与 `src/cli/menu.rs:70`、
   `scripts/install.sh:37` 的项目署名都写死为
   `github.com/xiaolingxiaoying/singbox-sub-me`。签名公钥也钉在 `scripts/install.sh`
   里（`docs/release-signing.md`）。在本仓库独立发 Release 之前，这套信任链是正确的；
   要改成新仓库，得同时换清单地址与安装脚本里的构建期公钥，不能只改一处。
2. **L3 验收套件有两条客户端腿。** `tests/acceptance/run.sh` 强制要求
   `SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT`（`verify-client.sh` 证明孤儿回收与 TUN 接线，
   `verify-sbcli.sh` 证明共享后台协议）。本仓库没有这两个二进制，脚本保持原样未改；
   `scripts/dev/build-acceptance-artifacts.sh` 会打印
   `branch: server-only workspace - skipping the client leg` 并只产出两个 sbctl 产物，
   随后 `run.sh` 会明确拒绝启动。要跑完整套件，请在 monorepo 里构建 sbtui/sbcli 并
   导出路径；`verify-bootstrap.sh`/`verify.sh`/`verify-real.sh` 这三条服务端断言本身
   不需要客户端。
3. **`vps-sub-meter` 这个名字在历史上是前身 Shell 项目。** `docs/implementation-plan.md`
   的总结里写着"吸收 `vps-sub-meter` Shell 脚本中的有效能力"——本仓库沿用了这个名字，
   但与那套 Shell 脚本没有代码继承关系，包名与二进制名仍是 `sbctl`。

## 验证状态（2026-10-07 实测）

本仓库独立跑过、有输出为证的门控：

| 探针 | 命令 | 结果 |
|---|---|---|
| Linux 全量门控 | `scripts/dev/wsl-gate.sh` | fmt 干净；workspace clippy（`--all-targets --all-features -D warnings`）干净；**sbctl lib 255、cli 125、json-merge 10** 全通过；`wsl gate passed` |
| 真核矩阵 | `scripts/dev/wsl-real-cores.sh` | sing-box 1.10–1.14 五核 + 固定 mihomo：`real-core matrix passed` |
| Windows 门控 | `cargo fmt --check` / clippy / test | fmt 与 clippy 退出码 0；**sbctl lib 248、cli 104、json-merge 7** 全通过 |
| CI 的 `--locked` 前提 | `cargo build --locked -p sbctl --no-default-features` | 退出码 0（缩减后的 `Cargo.lock` 自洽） |
| CI 的 python 腿 | `python3 -m unittest discover -s scripts -p 'test_*.py'` | Ran 2 tests，OK |
| CI 的独立信任锚腿 | `cargo test -p sbctl --no-default-features --test release_trust` | 1 passed |
| 改过的验收产物脚本 | `bash scripts/dev/build-acceptance-artifacts.sh` | 退出码 0；打印 `branch: server-only workspace - skipping the client leg`；只产出 `sbctl-linux-amd64` 与 `sbctl-test-signing` |
| 工作区成员 | `cargo metadata --no-deps` | `['json-merge', 'sbctl']`，无客户端 crate |
| 复制完整性 | `diff -r` src / tests / crates/json-merge / scripts/install.sh | 全部逐字节一致；文件数 src 58/58、tests 25/25、snapshots 13/13、ADR 32/32 |
| 索引换行符 | `git ls-files --eol` | 所有 `*.sh` 为 `i/lf`（带 `attr/text eol=lf`），安装脚本不会被 CRLF 破坏 |
| 文档链接 | 相对链接扫描 | 48 条中 5 条断链，全部指向客户端文件，逐条见上表 |

测试计数与源仓库**完全相同**（Windows 248/104、Linux 255/125），这是"服务端测试一个没丢"的直接证据；
同时 Linux 侧源仓库那 3 个 sbtui 既有失败在本仓库不存在，因为客户端 crate 不在这里。

**未**在本仓库跑过的：Docker systemd L3 验收（这台宿主机 Docker 守护进程没起，且缺 sbtui/sbcli 二进制）、
生产签名 Release 链（`release.yml` 未复制）。

