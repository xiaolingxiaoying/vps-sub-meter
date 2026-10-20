# P2：清除服务端覆写没有激活新配置

Status: needs-triage

位置：`src/cli/commands/config.rs:464`。

`config override clear server` 和包含 server 的 `clear all` 移除源文件后仅调用 regenerate。它会更新服务端工件和 `/etc/sing-box/config.json`，但不会重新加载或重启 sing-box。命令返回成功，运行中的服务仍采用已被删除的监听器/出站/路由等覆写。相比之下，托管部署的 override edit 会走配置激活与失败恢复事务。

## 隔离复现

`../reproduce.py` 的 `clear_server()`：创建带 ownership marker 的托管临时根目录，先应用服务端 `log.level=debug` 覆写；再清除 server。源文件被删除，active config 恢复默认日志级别，但 systemctl 调用日志为空，未进行任何 reload/restart。

## 修复方向

清除 server/all 时复用服务端编辑的激活、健康检查与恢复事务，并将覆写源文件备份纳入同一把操作锁；纯客户端清理可以继续只重新生成。
