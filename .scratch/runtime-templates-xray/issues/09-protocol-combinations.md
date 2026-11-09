# 09 扩展组合预设与客户端兼容导出

Status: ready-for-agent
Type: task
Blocked by: 无（候选生成）；06（托管激活）

用户于 2026-11-09 扩展原计划范围：新增 VLESS XHTTP Reality、VLESS XHTTP TLS、VLESS WS TLS 和 Trojan TCP TLS；客户端覆盖全部，按实际能力过滤而非生成伪兼容节点。此要求修订 spec 首版不包含 XHTTP 的原边界。

## 验收

- 类型化组合及参数校验、Xray 服务端和客户端生成、sing-box/Mihomo/URI/Shadowrocket 导出，明确版本能力及过滤原因。
- XHTTP 不能退化为 WS/TCP；不支持目标无兼容节点时失败，不输出空配置。
- 真实协议连接、各客户端内核与应用导入测试，不以配置检查替代真实连接。
- 完成 06 后把生成器接入统一节点/订阅事务、部署菜单、持久化凭据、证书权限与冲突检查；不是只提供候选导出。

## Comments

2026-11-09：类型化 Node/Preset 和 protocol list/check/export 候选入口已实现；生成器拒绝非法密钥、端口重用、错误凭据和不匹配的 Reality 设置。Xray 四组合配置检查通过，三个 TLS 组合本地 HTTP 真实连接通过。Reality 自签本地目标原型未通过；Mihomo 官方下载 504，真实校验待补。各 GUI App 导入和正式托管接入尚未完成。详见 [用户文档](../../../docs/protocol-presets.md)，本任务不标记完成。

2026-11-09 后续：用户要求四组合加入正式订阅并保留原 URL。已部署伴随 Xray、持久化源、Rust enable 与正式工件合并；9 条 URI 中原有 5 条完全保留。VPS 与 WSL 公网四组合转发、订阅再次生成和伴随服务重启通过。Reality/Xray 与 Mihomo TLS 的实测证据已补齐；GUI App、完整安装菜单和通用切换事务仍未完成，保留任务开放状态。
