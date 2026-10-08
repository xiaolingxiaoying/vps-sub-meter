# 04 更新回滚按路径恢复属主、权限与可执行位

Status: ready-for-agent

## 问题

`src/update.rs::restore` 用 `store.write_relative_locked` 重写每一条备份记录，进入
`src/config.rs::enforce_live_file_owner` 的兜底分支 `sbctl:sbctl 0600`。但回滚集合
（`MANAGED_PATHS` + `rollback_paths`）包含 5 个 systemd 单元、certbot deploy hook
以及 Direct 模式下的 pinned 证书副本；`restore` 只对 `usr/local/bin/` 补可执行位。

回滚后：pinned 私钥变 `0600 sbctl:sbctl` → `User=sing-box` 读不到 → 数据面重启失败且
不会自愈；deploy hook 掉 `+x` → certbot 续期后 hook 持续失败，pinned 证书不再刷新。

## 要做

- `src/config.rs::managed_ownership(relative)`：`etc/systemd/system/` → `root:root 0644`；
  `etc/letsencrypt/renewal-hooks/` → `root:root 0755`；`var/lib/sbctl/certificates/` →
  `root:sbctl-cert 0640`；保留既有三条与兜底。
- `src/update.rs::restore`：对 `usr/local/bin/*` **和** deploy hook 调用 `set_executable`；
  恢复过证书文件后重新 `chgrp -R sbctl-cert`。
- 抽出 `src/lifecycle.rs::grant_pinned_certificate_group(root)` 供安装与回滚共用
  （fixture root 为 no-op）。

## 验收

- 单测：`restore()` 恢复 deploy hook 的 0755。
- L3（`verify-real.sh`，live root，Direct 安装之后）：记录 pinned `privkey.pem` 与
  deploy hook 的 `stat`；`systemctl mask sbctl.service` 让更新健康检查失败；断言
  两者在回滚后与失败前完全一致。

## 实现

`src/config.rs`、`src/update.rs`、`src/lifecycle.rs`、`tests/acceptance/verify-real.sh`。

## 验证命令

```bash
cargo test --lib -- update::tests::a_rollback_restores_the_certbot_hook_executable_bit
```
