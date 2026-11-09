# 实施进度

## 2026-11-09：内核命令边界与 Xray 渲染基础

已实现：

- `src/core.rs` 定义 sing-box/Xray 序列化名称及结构化运行/检查参数，现有 sing-box 配置检查复用适配器。
- 配置检查不把内核原始诊断直接输出到日志，避免打印配置片段中的秘密；错误保留内核身份和退出状态。
- 临时检查文件使用 `.json` 后缀，兼容 Xray 格式自动识别。
- `src/core/xray.rs` 从 CanonicalNode 生成 VLESS Reality、VMess WS TLS 服务端配置，保留凭据/端口/路径；未验证协议明确拒绝。生成函数没有文件写入和服务激活副作用。
- `tests/xray_core.rs` 提供显式启用的真核配置检查，覆盖单协议/组合和 IPv4 策略；不等同真实连接或 systemd 验收。

工件研究：已取得官方 `v26.10.31` 发布元数据，Linux amd64 工件下载并校验 GitHub 摘要，通过版本执行；未把此步骤称作独立发布者签名验证。

| 官方工件 | 发布元数据 SHA-256 |
| --- | --- |
| Xray-linux-64.zip | e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214 |
| Xray-linux-arm64-v8a.zip | 1811c8f1da0c3b6dc707ebaf1c8390e987ba39859a8f0a7aadd8526157f2ec8d |

amd64 下载位于 WSL `/home/ly/ws/xray-probe/`，二进制未放入仓库。arm64 仅记录元数据，尚未下载运行。

验证结果：WSL fmt、全 workspace/all-features Clippy、workspace test-signing 回归通过（574 个非忽略测试）；另行显式运行的官方 Xray 真核用例通过，覆盖 6 份配置。既有 6 个 sing-box 真核用例未在本轮启用，systemd 生产验收也未执行。Windows 的 core 单元测试通过。曾并发构建不同 feature 覆盖测试二进制，导致 9 个 CLI 签名用例失败；已消除并发产物冲突并完整重跑通过。Xray 真核用例改用独立 target 目录。

尚未完成：01 的跨客户端真实连接、能力/迁移 ADR；02 的部署字段与完整校验解耦；03 外部模板；04 与活动部署的接入；05–08 下载/清单、事务、菜单和生产环境验收。因此不得将任何计划任务标记已完成，也尚未开放内核切换入口。

后续从 01 的跨客户端互通证据及迁移契约继续，随后完成 02/03。生产配置、服务、凭据与发布版本未修改。
