# 02 安装回滚不得删除事务外的管理二进制

Status: ready-for-agent

## 问题

`src/lifecycle.rs::rollback_fresh_installation` 的删除列表含
`usr/local/bin/sbctl`，而唯一豁免判据 `predates_transaction` 只认
`etc/sbctl/config.toml` 与 `var/lib/sbctl*`。该二进制由 `scripts/install.sh`
在事务开始前写入，`src/preflight.rs::existing_deployment_paths` 的冲突列表
也不含它，所以 `--replace-existing` 也不会备份它。

一次失败的安装会删除管理员唯一的 CLI：`ly` 悬空、`sbctl status/menu/uninstall`
全部不可用，Direct 模式下 certbot deploy hook 也一并失效。

## 要做

- `PreexistingState` 增加 `management_binary: bool`；`preexisting_state()` 从
  `usr/local/bin/sbctl` 的存在性采样。
- `predates_transaction()` 增加：`management_binary && relative == "usr/local/bin/sbctl"`。
- 回滚打印明确警告说明该文件被保留。
- **不要**把该路径加入 `existing_deployment_paths()`：`--purge` 之后残留的旧二进制
  不应阻止重新安装。

## 验收

- 单测：`management_binary: true` 时回滚保留；`Default`（本次事务创建）时删除。
- CLI 测试：预置 `usr/local/bin/sbctl` → 用 check 必失败的 `sing-box` 桩触发安装失败 →
  断言该文件内容不变。
- L3：`verify-real.sh` 的失败安装路径不得让 `sbctl` 命令消失。

## 实现

`src/lifecycle.rs`、`src/cli/commands/install.rs`、
`tests/cli/install.rs::a_failed_install_keeps_the_management_binary_and_rolls_back_the_certificate_directory`。

## 验证命令

```bash
cargo test --lib -- lifecycle::tests::a_rollback_keeps_a_management_binary_that_predates_the_transaction
cargo test --features sbctl/test-signing --test cli -- a_failed_install
```
