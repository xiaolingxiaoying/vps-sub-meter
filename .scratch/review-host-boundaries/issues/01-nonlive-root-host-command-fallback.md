# 非生产 root 缺少命令替身时回退宿主机
Status: resolved

Runtime::live(nonlive_root) 允许 PATH 回退；lifecycle 的 systemctl、账户管理命令也各自直接回退 PATH。隔离部署或测试可能因此启停宿主机服务，调用真实 sysctl、UFW 或账户管理命令。需统一约束命令边界并覆盖缺失及正常替身场景。

## 结果

`d25f562` 统一 Runtime 命令边界，非 `/` 根目录缺少替身时明确失败。旧 Actions v0.0.16 与新生产产物在 VPS 命名空间中对比，确认 restart、BBR 和 renew 的宿主机 PATH 标记只被旧版触及；提供 root 内替身后两版均按预期执行。真实根目录服务启停、API 和覆写回归通过。详见 [验收记录](../verification.md)。
