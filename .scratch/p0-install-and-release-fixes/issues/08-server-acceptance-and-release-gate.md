# 08 服务端 systemd 验收可跑 + 发布门禁

Status: ready-for-agent

## 问题

`tests/acceptance/run.sh` 硬要求 `SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT`，而本仓库没有这两个
二进制，于是 `ci.yml` 里唯一能验证安装事务的 `server-acceptance` job 被整段注释掉。
P0-2/P0-3/P0-4/P0-5 的问题全部只有真机 systemd 安装才会暴露。

`Cargo.toml` 的 `test-signing` feature 会让二进制信任公开的开发公钥，而本仓库没有
发布门禁兜底。

## 要做

- `tests/acceptance/run.sh`：`SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT` 改为可选；未设置时打印
  `branch: server-only workspace - skipping the client legs` 并跳过
  `verify-client.sh`/`verify-sbcli.sh`；仍强制 `SBCTL_ARTIFACT`/`SBCTL_TEST_ARTIFACT`。
- `ci.yml` 恢复 `server-acceptance` job（`--privileged` + cgroup 挂载，debian:12-slim /
  ubuntu:22.04 / ubuntu:24.04），依赖 `test`，在 job 内构建 release 与 test-signing 两个产物。
- 新增 `scripts/dev/release-trust-anchor-check.sh`：行为化门禁——用开发种子签一个合法
  manifest，断言发布二进制拒绝它、且拒绝原因不是"未配置锚点"，并断言 schema 2 的 manifest
  仍被拒绝。
- `ci.yml` 的构建 job 注释写明"该产物不含锚点，不可用于 `sbctl update`"。

## 验收

- 本地/CI 跑 `sh tests/acceptance/run.sh`（不带客户端变量）：打印跳过说明，三条服务端断言全过。
- `bash scripts/dev/release-trust-anchor-check.sh <生产构建>` 通过；
  对 `--features test-signing` 构建或无锚点构建必须失败（退出 1）。

## 实现

`tests/acceptance/run.sh`、`.github/workflows/ci.yml`、
`scripts/dev/release-trust-anchor-check.sh`。
