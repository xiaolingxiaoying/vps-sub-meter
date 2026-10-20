# P1：观测接口失败恢复重新启用旧订阅凭据

Status: resolved

位置：`src/cli/commands/update.rs:327–343`，`sing_box_api` / `apply_and_restart`。

观测接口启停在锁外读取配置，`apply_config_transaction` 只在生成和写入期间持锁，返回后立即释放。后续准备服务、重启、健康检查和恢复均没有共同锁。另一终端可在这个窗口成功轮换或撤销订阅凭据，随后观测接口操作失败又用完整旧快照覆盖最新配置，重新启用已失效的凭据。

## 隔离复现

`../reproduce.py` 的 `api_rollback()`：暂停 `sing-box api enable` 的首个 systemctl restart，此时已完成发布并释放锁；执行 `credential rotate` 并确认成功且凭据改变；让第一个 restart 失败。API 命令退出 2，但磁盘凭据恢复为轮换前旧值，新凭据丢失。

## 修复方向

在读取当前配置前获取操作锁，保持到激活和恢复结束，改用持锁事务接口。此处与 ADR-0033 的凭据失效及配置事务约束冲突；上一轮只修复了向导路径，没有覆盖这个入口。

## Comments

由 `60c61d1` 修复：观测接口启停持锁完成整个读写、服务激活和恢复。CLI 回归覆盖并发默认凭据轮换和命名凭据撤销。Actions 构建的候选程序在 VPS 的隔离回归通过。
