# 06 验收、CI 与交付

Status: ready-for-agent

Windows 和 WSL 常规回归、真实内核配置/转发、伪终端、测试 VPS 三模式/故障恢复/实际重启/订阅/权限验收已通过，脱敏结果见 verification.json。固定内核和 UDP 检查已接入 CI。仍需完整三发行版 systemd 矩阵、GUI 导入及正式签名发布门禁，故此任务保持 ready-for-agent。

## Comments

2026-11-14：用户已授权覆盖测试 VPS 配置；保留 root 私有原部署备份，不在报告中包含订阅凭据、节点秘密或 SSH 密码。

2026-11-14：当前候选与发布前限制记录于 docs/managed-verification-2026-11-14.md；本轮未合并稳定分支、未发布正式版本。
