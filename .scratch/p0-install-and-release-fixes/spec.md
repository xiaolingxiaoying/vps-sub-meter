# P0 安装与发布修复（spec）

Status: ready-for-agent

## 目标

修掉第三轮审查（[`docs/project-review-round-3-2026-10-08.md`](../../docs/project-review-round-3-2026-10-08.md)）里的七条 P0，让本仓库具备**自洽的安装/更新链路**：文档指向的 Release 由本仓库自己生产，安装失败不再损坏主机，更新回滚不再改变受管文件的权限语义。

## 范围

- 在本仓库新增签名发布流水线（Release 产物：两个架构的 `sbctl`、两个架构的 sing-box 运行时、两个签名 manifest、`install.sh`、`SHA256SUMS`）。
- 修正安装事务边界：证书写入纳入事务、回滚不删事务外的管理二进制。
- 修正更新回滚的属主/权限/可执行位。
- 官方内核下载 fail-closed，归档只解出预期成员。
- `install.sh` 先决策、后落盘。
- `sbctl install` 早退判据覆盖全部参数。
- 让服务端 systemd 验收在本仓库可跑（两条客户端腿可选 + 恢复 CI job），并加发布门禁。

## 不做

P1/P2 的其余条目（`switch-mode`、`--bind`、总览页流量口径、manifest 序列/有效期、自签私钥符号链接、`parse_grace` panic、限流淘汰、候选 TOCTOU、action pin、版本号/LICENSE、官方内核的 TOCTOU 之外的加固）只记录在审查报告里，本轮不实现。

## 决策记录

1. **发布信任链落在本仓库**，不再指向 `singbox-sub-me`。生产公钥走仓库变量 `SBCTL_RELEASE_PUBLIC_KEY_HEX`（编译期 + `install.sh`），私钥只在 `release` Environment 的 `SBCTL_SIGNING_SEED`。
2. **随包 sing-box 运行时钉在 `scripts/release-runtime-pins.txt`**（版本 + 逐架构 sha256 + 兼容区间），发布构建先校验摘要再签名，`generate-manifest.sh` 从同一文件读取兼容区间。
3. **发布用草稿态**：`gh release create --draft` → 上传全部资产 → `--draft=false`。半途失败留下可删除重跑的草稿，已发布字节不被覆盖。
4. **回滚判据用"是否本次事务创建"**，不用路径白名单：`PreexistingState` 记录 `management_binary`，由 `install.rs` 在事务开始前采样。
5. **事务开始点前移到工件生成之前**：SelfSigned 证书是工件生成期的副作用，必须被回滚覆盖。
6. **官方内核摘要缺失一律 fail-closed**，错误信息指向 `--manifest` / `--sing-box-bin`；解压只解出 `sing-box-<version>-linux-<arch>/sing-box`。
7. **保留"裸调用 + 非 TTY = 只读预检"契约**（`scripts/install.sh` 依赖它），用 `InstallOptions::is_bare()` 的穷尽解构保证新字段不会漏判。
8. **本仓库独立维护**，修复不回灌 `singbox-sub-me`。

## 验收标准

1. 失败安装之后：`/usr/local/bin/sbctl` 仍在且可执行、`var/lib/sbctl/certificates` 不存在、紧接着重跑安装成功。
2. 失败的内核更新之后：pinned 私钥仍是 `0640 root:sbctl-cert`、deploy hook 仍是 `0755`。
3. GitHub 未提供资产摘要时内核下载失败，且错误信息给出两条替代路径；含 `..`/绝对路径成员的归档被拒绝。
4. `sbctl install` 携带任一端到端参数时不再静默空转；裸调用 + 非 TTY 仍只打印预检结果。
5. 安装器在"保留并退出"路径下不改变 `/usr/local/bin/sbctl` 的字节。
6. 文档里的安装命令能在一个真实 Release 上跑通；发布出的 `install.sh` 通过 `prepare-installer.py` 注入生产公钥。
7. `tests/acceptance/run.sh` 在无客户端二进制时打印跳过说明并跑完三条服务端断言。

## 回归门

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --features sbctl/test-signing`
- `cargo test --locked -p sbctl --no-default-features --test release_trust`
- `python3 -m unittest discover -s scripts -p 'test_*.py'`
- `sh tests/acceptance/run.sh`（CI `server-acceptance` job，三发行版）
- 发布链演练：fork 上配好 Variable/Environment，推 `sbctl-v*`，确认草稿资产齐全后发布，再用发布出的 `install.sh` 装一台干净容器。

## 工单状态

| # | 工单 | 实现 | 验证 |
|---|---|---|---|
| 01 | `issues/01-release-pipeline.md` | 已落地于本工作树 | 需在 fork 上跑一次真实发布 |
| 02 | `issues/02-rollback-keeps-management-binary.md` | 已落地 | 单测 + CLI 测试已过 |
| 03 | `issues/03-certificate-write-inside-transaction.md` | 已落地 | CLI 测试已过 |
| 04 | `issues/04-rollback-ownership-and-executable-bits.md` | 已落地 | 单测已过；L3 `verify-real.sh` 待跑 |
| 05 | `issues/05-official-kernel-fail-closed.md` | 已落地 | 单测已过 |
| 06 | `issues/06-installer-decides-before-writing.md` | 已落地 | `verify-bootstrap.sh` 待跑 |
| 07 | `issues/07-install-bare-invocation.md` | 已落地 | CLI 测试已过 |
| 08 | `issues/08-server-acceptance-and-release-gate.md` | 已落地 | CI job 待首跑 |
