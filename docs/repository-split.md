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

## 不包含什么（以及为什么）

- `crates/sbcli`、`crates/sbtui`、`crates/sbgui`、`crates/client-core`：客户端栈。
  拆分前已验证 `src/` 与 `tests/` 对它们**只有注释级引用**（`src/lifecycle.rs:761,775`、
  `src/subscription/render/singbox.rs:132`），没有任何代码或构建依赖，所以删除后服务端自洽。
- `prototypes/`、`packaging/windows/`、`scripts/sbgui-*`、`scripts/winvm`、
  `scripts/dev/wsl-signal-exit.sh`：GUI/TUI 专用资产。
- `.github/workflows/release.yml`：该工作流的 acceptance/publish 任务串了
  `build-sbtui`、`build-sbgui-qml`、MSI 与共享 daemon 产物，照搬必然失败。
  若本仓库要独立发 Release，需按 `docs/release-signing.md` 重新设计。
- 客户端主题文档：`client-core-control-api.md`、`client-description.md`、
  `DESIGN.md`（QML 视觉系统）、`gpui-*`、`qml-*`、`sbgui-*`、
  `code-review-2026-09-25.md`、`project-review-2026-09-24.md`、
  `project-review-round-3-2026-10-02.md`（后两篇的主体是客户端）、
  `docs/research/` 里的 gpui/ratatui/client 篇。
- `.scratch/`（本地工单目录）、`target/`、`dist/`。

保留的服务端文档里仍有指向已删除文件的链接（例如 ADR 提到 `crates/sbgui/…`、
`docs/round-2` 报告含客户端章节）。这些是决策记录原文，**故意不改写**，断链指向的是
`singbox-sub-me` 里的对应文件。

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

## 验证状态

拆分时本仓库独立跑过的门控（详见 README"验证"一节）：Linux 的 fmt / clippy / 全量测试、
真核矩阵、以及 Windows 侧构建。**未**在本仓库跑过的：Docker systemd 验收（宿主机 Docker
守护进程未启动）、生产签名 Release 链。
