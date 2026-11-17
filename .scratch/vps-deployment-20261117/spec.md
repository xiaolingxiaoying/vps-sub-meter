# 三 VPS 部署与问题核查

依据 2026-11-17 外部诊断执行 GTT、NTT、VMISS 的现有部署更新，保留订阅地址、节点凭据、监听端口、节点组、流量账期及已有 DNS 定制，处理可安全复现的问题。

部署、构建、验证和限制见 [完整记录](../../docs/vps-deployment-2026-11-17.md)。机器证据见 [results.json](results.json)。本地敏感材料保持在 ignored 的 managed-vps-private 目录，禁止附到后续 issue。

## 完成项

- 三台最终候选部署、原配置与身份保留、真实代理及订阅验收。
- DNS/IPv4 持久覆写、sing-box 出站防护和 Mihomo Fake-IP 模板修复。
- GTT 废弃 UFW 规则移除、VMISS 日志归档及容量上限。

## 后续事项

Xray 出站防护见独立工单。VMISS 直连 TCP 属于当前网络路径故障，不能作为程序部署已修复的事项关闭。
