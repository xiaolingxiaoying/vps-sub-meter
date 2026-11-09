# 04 Xray 服务端生成与协议适配

Status: ready-for-agent
Type: task
Blocked by: 01, 02, 03

## 工作

从统一节点模型生成 Xray 服务端配置，实现 VLESS Reality 与 VMess WebSocket 的证书、传输和凭据映射；原始配置使用结构化序列化。实现目标版本能力拒绝逻辑和 Xray 专属服务端模板/分层覆写，拒绝凭据、监听器身份及证书等受保护字段被篡改。

客户端继续使用统一节点模型，双方共同能力保持节点字段和 URI。非共同协议默认拒绝，不自动替换成其他协议。对 server_clash_api 等 sing-box 功能明确产生兼容性诊断，不写入 Xray 配置。

## 验收

- 生成配置通过固定版本官方 Xray run -test。
- VLESS Reality 和 VMess WS TLS 分别使用固定 sing-box/Mihomo 客户端真实连接；验证 UUID、Reality 公钥、flow、WS path/SNI 与证书策略。
- 两种服务端下对应客户端字段一致，不重新生成凭据。
- 不支持协议/版本、非法覆写和无效证书路径报出不含秘密的明确错误。
