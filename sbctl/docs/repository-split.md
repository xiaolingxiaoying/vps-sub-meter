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
  `docs/research/` 少 10 篇（GPUI 相关 7 篇 + `ratatui-study-for-sbtui.md` + 两篇客户端综述；源提交共 14 篇、副本保留 4 篇）；
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

1. **自更新和安装已接入本仓库 Release。**
   `src/update.rs` 与 `scripts/install.sh` 从
   `github.com/xiaolingxiaoying/vps-sub-meter/releases` 获取签名清单；
   [`../.github/workflows/sbctl-release.yml`](../.github/workflows/sbctl-release.yml) 将生产公钥编入
   amd64/arm64 二进制，用 `release` Environment 中的私钥签名并用发布二进制回验清单，
   随包 sing-box 的版本与逐架构 sha256 钉在 `scripts/release-runtime-pins.txt`。
   仓库内的 `scripts/install.sh` 仍是未配置公钥的模板，部署应使用 Release 附带的
   `install.sh`。复制到其他仓库时，必须同时调整 Release URL、生产公钥和签名密钥配置。
   发布链的四个阶段与失败模式见 `docs/release-signing.md`。
2. **L3 验收套件的两条客户端腿是可选的了。** `tests/acceptance/run.sh` 不再强制要求
   `SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT`：未设置时打印
   `branch: server-only workspace - skipping the client legs`，跳过 `verify-client.sh`
   （孤儿回收与 TUN 接线）与 `verify-sbcli.sh`（共享后台协议），只跑
   `verify-bootstrap.sh`/`verify.sh`/`verify-real.sh` 三条服务端断言。要跑完整套件，
   请在 monorepo 里构建 sbtui/sbcli 并导出两个路径。CI 的 `server-acceptance` job
   按无客户端变量的方式跑三条服务端断言。
3. **`vps-sub-meter` 这个名字在历史上是前身 Shell 项目。** `docs/implementation-plan.md`
   的总结里写着"吸收 `vps-sub-meter` Shell 脚本中的有效能力"——本仓库沿用了这个名字，
   但与那套 Shell 脚本没有代码继承关系，包名与二进制名仍是 `sbctl`。
4. **审查结论见 [`project-review-round-3-2026-10-08.md`](project-review-round-3-2026-10-08.md)。**
   七条 P0（安装事务边界、更新回滚权限、官方内核降级通道、发布链缺失等）已修复，
   P1/P2 条目逐条附证据与建议，仍开放。

4. **拆分时分层覆写的 CLI 仍是旧模型，生成逻辑与人工运维入口不对齐。** ADR-0029 和 `src/override_template.rs` 已支持客户端两种目标的基础文件与 `.d/` 层，以及 `sing-box-server.json` / `sing-box-server.d/` 服务端目标；当时 CLI 的 `show/edit/clear/validate` 有以下边界：

   | 命令 | 抽取时实现 | 抽取时缺口 |
   |---|---|---|
   | `config override show` | 只列 `sing-box-override.json` 与 `clash-override.yaml` 两个客户端基础文件 | 不展示客户端 `.d/` 层或服务端目标；输出的合并说明也没写 `rules_mode` 与按 `tag` / `name` 合并的数组规则。README 的“已知缺口”已记录前两类展示问题及 `rules_mode`。 |
   | `config override edit` | `CliOverrideTarget` 只有 `sing-box` 和 `clash` | 没有 `edit server`，也没有按文件名编辑某个 drop-in 层；服务端扩展只能由管理员手工创建/编辑文件。 |
   | `config override validate` | `Overrides::load` 会解析三种目标；有内核时命令只对 `subscription-sing-box-full.json` 执行 `sing-box check` | 不会对合并后的 `sing-box-server.json` 执行真核检查。正常 `regenerate_current` 路径会检查服务端工件，并在存在覆写时检查合并后的客户端 full profile；两条 CLI 路径的验证覆盖不同。 |
   | `config override clear` | 只删除两个客户端基础文件，然后重新生成工件 | 客户端 `.d/` 目录和服务端基础文件/层会保留，所以这个命令不是清空全部覆写。 |

   `tests/cli/config_topics.rs` 中唯一直接调用 `config override` 的专项 CLI 测试是 Unix 下的编辑器回退测试；它覆盖编辑器探测和客户端样例文件创建，没有覆盖 `show`、`validate`、`clear` 或服务端目标。拆分仓库排除了 `.scratch/`，因此原报告中的“W5”编号在此没有可回查的工单来源；本节按当前代码行为记录。

   这意味着在拆分报告生成时，README 只写到了 `show` 展示和 `edit server` 缺失，还没有说明 `validate` 的服务端真核检查空档、`clear` 的保留范围和 CLI 测试覆盖空白。

   **2026-10-08 修复状态：** `show` 现在列出三个目标和有效分层文件；`edit` 支持 `server` 与 `--layer <文件名>`；`validate` 区分未初始化与配置损坏，并对服务端和客户端配置都执行真核检查；`clear` 默认清除两种客户端覆写，`clear server` / `clear all` 显式清理服务端目标或全部目标，验证失败会回滚文件。

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

## 二次复核补充（2026-10-07）

- **文档计数已校正**：`2b2a411` 的 `docs/research/` 有 14 篇，副本有 4 篇，差集为 10 篇。GPUI 相关文件是 7 篇，不是 8 篇；再加 1 篇 `ratatui-study-for-sbtui.md` 和 2 篇客户端综述，合计 10 篇。
- **源分支指针复核**：源仓库的 `fix-ui` 分支仍指向 `2b2a411`；本次只读复核时，源仓库当前 checkout 是 `main`（`a4298f1`），工作树干净。正文中“源仓库 fix-ui 状态”描述的是拆分报告生成时的快照，不代表源仓库当前 checkout 分支。
- 本节是对报告和代码的静态复核；没有重跑构建、测试或 L3 验收，也没有修改源仓库。

## 覆写 CLI 修复（2026-10-08）

本轮修复落在当前 `vps-sub-meter` 工作树：补全三目标与分层文件的 `show/edit/clear`，让 `validate` 检查服务端与客户端工件，并让覆写目录符号链接、配置解析错误和清理失败走显式错误/回滚路径。源仓库 `singbox-sub-me` 未修改。`cargo fmt --all -- --check`、`cargo check --locked -p sbctl --no-default-features` 和 `cargo clippy --locked -p sbctl --all-targets --all-features -- -D warnings` 均通过；本轮没有新增或运行测试。

