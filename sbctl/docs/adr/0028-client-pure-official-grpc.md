# 客户端纯官方 gRPC 控制面

- **状态：** 已接受
- **日期：** 2026-10-04
- **取代：** ADR-0026 中"对 1.10–1.13 保留 Clash API"的兼容条款，以及 ADR-0027 中"现有 UI 仍兼容 1.10–1.13"的承诺
- **相关：** ADR-0026（官方 gRPC 控制面与订阅限时）、ADR-0027（共享客户端后台与 sbcli）

## 背景

ADR-0026 引入官方 `type: api` StartedService gRPC 控制面时，为保证老内核用户平滑过渡，保留了 sing-box 1.10–1.13 的 Clash REST API 兼容传输；ADR-0027 延续了这一承诺。该兼容层带来三类持续成本：

1. **双传输分发维护**：`ControlApi` 需要为每个操作维护 Clash REST 与 gRPC 两条分支，`control_api_kind()` 按内核版本分派，新增能力时必须双写或显式裁剪。
2. **字符串耦合**：Clash 分发层把 sing-box 内核的 REST JSON 结构固化进客户端领域模型（`/proxies`、`/connections`、`/traffic` 流式 JSON 拼接解析等），上游对 Clash API 的任何调整都会波及客户端。
3. **双传输测试负担**：引擎测试需要同时维护 Clash HTTP mock 与 tonic mock 两套底座，且自定义单节点测速的"兼容测速通道"（三端口预留、独立 secret）复杂度高、价值低。

sing-box 1.14+ 的官方 gRPC API 已覆盖客户端所需的全部控制能力（组测速 URLTest、连接管理、流量/日志/模式流），官方自身也已宣布 Clash API 进入维护裁剪阶段。

## 决策

1. client-core 仅支持 sing-box 1.14+ 内核，仅使用官方 StartedService gRPC（API version 4）作为控制传输。
2. 删除 Clash REST 传输层与自定义单节点 URL 测速的兼容通道（`ControlApi::delay_with` 的 Grpc 兼容分支、三端口预留逻辑）。
3. 启动时检测到 1.13 及以下内核版本，返回可操作的中文升级指引错误（含版本要求），不再回落 Clash 传输。
4. `close_all_connections` 走 gRPC 原生 `CloseAllConnections` RPC。
5. 客户端领域类型（`model.rs`）与传输层解耦，供两个 UI 复用；类型语义以 gRPC 映射为准。
6. `crates/client-core/proto/` 为 vendored 官方 proto：本仓库不做上游自动同步检查（已知风险，升级内核支持版本时需人工比对上游 `daemon/started_service.proto`）。

## 后果

- 内核 1.10–1.13 的用户必须升级到 1.14+ 才能使用本客户端；启动报错需给出明确版本要求与升级途径。
- Clash REST 客户端、`/traffic` `/memory` 流式 JSON 解析、三端口预留与独立 delay secret 全部移除；启动配置不再注入 Clash REST 端点。
- 单一传输后，`ControlApi` 分发层随之取消（client-core 直用 `GrpcApi`），引擎测试底座统一为 tonic mock。
- 自定义单节点 URL 测速在官方 gRPC 协议中没有对应 RPC，保持不可用；组测速使用原生 URLTest。网络质量测试（NetworkQualityTest）与 STUN 检测使用官方 RPC 补齐（见票 09）。
- ADR-0027 的 sbcli"仅支持 1.14+"边界扩展为所有前端统一边界。

## 相关决策与文档

- [ADR-0026：官方 gRPC 控制面与订阅限时](0026-client-official-grpc-and-subscription-bounds.md)
- [ADR-0027：共享客户端后台与 sbcli](0027-shared-client-daemon-and-cli.md)
- [client-core 内核控制契约](../client-core-control-api.md)
