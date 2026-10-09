# 03 证书写入必须落在安装事务内

Status: ready-for-agent

## 问题

`src/cli/commands/install.rs` 先调 `generated_artifacts_for_kernel()`，再
`check_sing_box_config()`，之后才把 `installation_started` 置真。而工件生成在
SelfSigned 模式下会经 `src/subscription/render/singbox.rs::ensure_self_signed_certificate`
把证书写到 `/var/lib/sbctl/certificates/<sni>/`。

`src/preflight.rs::existing_deployment_paths` 把 `var/lib/sbctl/certificates` 列为
existing deployment，所以一次 `sing-box check` 失败就会让**后续每一次安装都被拒绝**，
只能手工 `rm -rf`。

## 要做

- 把 `installation_started = true` 前移到 `state_before_install` 捕获之后、
  `generated_artifacts_for_kernel()` 之前。
- 保持回滚范围不变（`remove_data_directory_keeping_lock` 已覆盖 `certificates/`）。

## 验收

- CLI 测试：check 失败的安装结束后 `var/lib/sbctl/certificates` 不存在，且
  `etc/sbctl/config.toml` 与 ownership marker 都不存在。
- CLI 测试：同一 fixture 紧接着用可用桩重跑安装必须成功（证明 preflight 不再被残留目录拒绝）。

## 实现

`src/cli/commands/install.rs`、
`tests/cli/install.rs::a_failed_install_can_be_retried_without_manual_cleanup`。

## 验证命令

```bash
cargo test --features sbctl/test-signing --test cli -- a_failed_install
```
