# 测速必须经过所选节点

Status: resolved
Created: 2026-11-16
Type: task

## 问题

`node probe` 为 curl 指定临时 SOCKS 客户端，但未覆盖继承的 `NO_PROXY` / `no_proxy`。当它们匹配测试目标或为 `*` 时，curl 会直接访问目标，失效节点也可能返回成功。

## 修复与验证

在测速请求中显式使用 `--noproxy ''`，使请求始终通过临时节点客户端；不修改用户环境或部署配置。

新增 `tests/probe.rs`：使用真实 sing-box，将节点指向只支持普通 HTTP 的端点，同时给 CLI 子进程设置两个绕过变量。修复前错误返回 HTTP 204；修复后节点检查失败，目标未收到直连 GET。该回归已接入真实内核 CI 和 `scripts/dev/verify-managed.sh`。

两台授权 VPS 的共 14 个节点均在设置 `NO_PROXY=*`、`no_proxy=*` 的子进程环境中实际返回 HTTP 204；部署设置、订阅地址、URI / Base64 URI / Shadowrocket 字节及已有节点组均保持。详情见 [双 VPS 验收](../../../docs/two-vps-verification-2026-11-16.md)。
