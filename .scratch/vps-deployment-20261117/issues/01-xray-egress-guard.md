# 为 Xray 提供持久的出站目标防护

Status: ready-for-agent
Type: task

## 问题

当前 managed Xray 服务器生成器仅生成入站及 direct freedom 出站，尚无 sing-box server overlay 对等入口。VMISS 的两个 XHTTP 节点因此未获得本轮 sing-box 的私网、回环、链路本地及 Fake-IP 拒绝规则。代理凭据持有者仍可提交这类目的地址；部署是单管理员模式，节点凭据不能被视为内网访问隔离边界。

## 验收

- 明确出站策略的配置边界与兼容性，不静默接管已有用户定制。
- 在生成与事务发布阶段持久生成 Xray 路由/拒绝出站，不能只手改活动 JSON。
- 真实内核验证字面私网地址、解析到私网的域名与 Fake-IP 目标被拒绝，正常公网 HTTPS 转发继续通过。
- 配置校验失败保留原服务、订阅、凭据和账期；恢复与重生成后防护仍在。

## Comments

2026-11-17：本轮保留现有 Xray 配置与进程，未将 sing-box 的防护验收扩称为 Xray 也已覆盖。证据见 [部署记录](../../../docs/vps-deployment-2026-11-17.md)。
