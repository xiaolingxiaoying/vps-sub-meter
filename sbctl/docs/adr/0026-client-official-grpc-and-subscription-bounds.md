# 官方 gRPC 控制面与订阅限时

> **2026-10-04 注记（ADR-0028）**：本文"对 1.10–1.13 保留 Clash API"的兼容条款已被
> [ADR-0028](0028-client-pure-official-grpc.md) 取代——客户端仅支持 1.14+ 内核、仅官方 gRPC 传输，
> Clash REST 兼容层与自定义单节点测速兼容通道已删除。其余订阅限时与安全边界仍然有效。

sing-box 从 1.14.0 提供 `type: api` 的 gRPC 服务，采用 `authorization: Bearer <secret>`。client-core 对 1.14+ 使用该官方 StartedService API，对 1.10–1.13 保留 Clash API。参考：[官方 API 文档](https://sing-box.sagernet.org/configuration/service/api/)。

本次支持现有控制功能和内核诊断，不扩展所有平台工具 RPC。控制端点仅绑定 loopback，使用每个实例独立的 secret，关闭 dashboard；API version 不匹配或鉴权失败必须显式失败。1.14+ 不再注入 Clash REST 延迟入口；只提供已配置 URLTest 组的默认 URL 测速，自定义单节点 URL 测试显式不可用。Dashboard 默认关闭，只能显式启用。

订阅抓取使用系统/环境代理，控制 API 直连 loopback。订阅 full/node 的兼容回退仅适用于 HTTP 404/410，不能隐藏 TLS、代理、鉴权或超时故障。服务器为握手与连接设置独立限时，证书解析离开连接接受路径，诊断与计数不暴露凭据。

共享后台的所有权、独立入口和迁移见 [ADR-0027](0027-shared-client-daemon-and-cli.md)。sbcli 仅接受 1.14+；TUI/GUI 继续兼容 1.10–1.13。远程 StartedService 只管理运行态，不能启停普通内核或修改配置。直连订阅支持 URL userinfo 的 Basic Authentication；带凭据的 URL 不允许通过下载镜像转发。
