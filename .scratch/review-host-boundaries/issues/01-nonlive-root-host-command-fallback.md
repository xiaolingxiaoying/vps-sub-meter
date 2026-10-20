# 非生产 root 缺少命令替身时回退宿主机
Status: claimed

Runtime::live(nonlive_root) 允许 PATH 回退；lifecycle 的 systemctl、账户管理命令也各自直接回退 PATH。隔离部署或测试可能因此启停宿主机服务，调用真实 sysctl、UFW 或账户管理命令。需统一约束命令边界并覆盖缺失及正常替身场景。
