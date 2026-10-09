# Acceptance fixture boundary

`verify.sh` is a black-box acceptance flow: it invokes only the administrator-visible
`sbctl` CLI and the subscription HTTP endpoint. `fixture.sh` supplies isolated host
state under `$work/<name>` for `/etc`, `/proc`, `/sys`, systemd command shims and
certificates. A fixture command is rooted below `usr/bin` and never falls back to the
container's real command.

The shell flow is suitable for local development and CI smoke checks. Production
support is established only by `run.sh` in a Debian/Ubuntu VM or equivalent real
systemd environment. WSL2 is a Development host for compilation, Rust tests and
simulated CLI checks; it is not evidence for the Production host release gate.

签名测试现在需要显式 `test-signing` 构建，通过 `SBCTL_TEST_ARTIFACT` 传入；生产工件仍通过 `SBCTL_ARTIFACT` 传入。两者不得混用，详见 [签名与验收隔离](../../docs/release-signing.md)。

`run.sh` 跑三发行版（debian:12-slim、ubuntu:22.04、ubuntu:24.04），每个容器依次执行
`verify-bootstrap.sh`（安装器：签名校验、参数转发、以及“已有部署时先决策后落盘”）、
`verify.sh`（fixture root 的事务、更新回滚点、卸载）与 `verify-real.sh`（真实 systemd 安装、
非 root 服务、socket activation、更新回滚后的权限与可执行位）。

`SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT` 是**可选**的：本仓库只有服务端 crate，未设置时脚本打印
`branch: server-only workspace - skipping the client legs` 并跳过 `verify-client.sh` 与
`verify-sbcli.sh`；设置后跑完整套件。CI 的 `server-acceptance` job 以无客户端变量的方式运行。
