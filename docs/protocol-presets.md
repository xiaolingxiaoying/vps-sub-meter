# 协议组合候选配置

新增四个预设：`vless-xhttp-reality`、`vless-xhttp-tls`、`vless-ws-tls`、`trojan-tcp-tls`。协议、传输和安全层由预设限定，不接受任意拼接的 transport/security/flow 字段。

当前入口用于生成、校验候选配置，**尚未接入托管部署、安装菜单或订阅 HTTP 路由**。执行下面的命令不会把节点加入已有部署，也不会启动/停止服务。现有 sing-box 节点配置保持原状。

## 使用

复制 [JSON 示例](../examples/protocol-presets.json) 为私有文件，替换域名、端口和全部示例凭据。示例中的 UUID/密码是公开演示数据，不能用于正式部署。Unix 下为包含凭据的输入和导出文件设置 0600 权限。

```bash
sbctl protocol list
sbctl protocol check --input nodes.json
sbctl protocol check --input nodes.json --xray-bin /path/to/xray \
  --certificate /path/to/fullchain.pem --private-key /path/to/privkey.pem

sbctl protocol export --input nodes.json --format xray-server \
  --certificate /path/to/fullchain.pem --private-key /path/to/privkey.pem
sbctl protocol export --input nodes.json --format xray-client
sbctl protocol export --input nodes.json --format sing-box
sbctl protocol export --input nodes.json --format mihomo
sbctl protocol export --input nodes.json --format uri
sbctl protocol export --input nodes.json --format shadowrocket
```

`check` 不带二进制时仅校验结构、UUID、Reality 密钥配对及端口/名称唯一性，不声称内核或连接可用。指定 Xray 二进制后检查完整服务端配置。TLS 服务端生成必须提供证书路径；客户端默认验证证书，不支持自动关闭验证。

`export` 有意将节点凭据输出到 stdout；普通校验消息和错误不会打印这些秘密。Xray 服务端产物包含完整入站和 direct 出站；客户端 JSON/YAML 是节点片段，不包含本地代理监听、代理组或完整分流策略，也不能直接覆盖正在使用的客户端配置。URI 是明文链接列表，用于支持该组合的导入器，客户端应用版本仍需单独验证。

## 导出兼容边界

| 目标 | XHTTP | VLESS WS TLS / Trojan TCP TLS |
| --- | --- | --- |
| Xray / v2rayN / v2rayNG 的 Xray 内核 | Xray 26.10.31 的 XHTTP TLS 和 Reality 均完成 VPS 数据通路验证；GUI 导入待测 | 两种组合完成 Xray 到 Xray 的 VPS HTTP 转发 |
| 官方 sing-box 已支持版本 | 跳过并提示 | sing-box 1.14.2 客户端到 Xray 服务端的 VPS HTTP 转发通过 |
| 保守 Mihomo/Clash 导出 | 跳过并提示 | Mihomo 1.19.33 的 VPS HTTP 转发通过；不承诺所有旧 Clash 内核支持 VLESS |
| 新版 Mihomo XHTTP | `--format mihomo-xhttp` 显式生成 XHTTP TLS，1.19.33 已互通；过滤当前 Xray 基线的 Reality 节点 | 生成 YAML 节点，已完成 VPS HTTP 转发 |
| Shadowrocket 兼容链接 | 保守跳过，尚未验证具体 App 版本 | 生成对应 URI，App 导入待测 |

全部节点被过滤时返回失败且不输出空订阅。过滤消息只写 stderr，不混入配置正文。新版 Mihomo 官方文档已列出 XHTTP，但不将此结论套用到旧版 Clash 或所有客户端应用；实验性导出不会替代现有保守导出。

Reality 节点使用同一结构，将 `preset` 设为 `vless-xhttp-reality`，并加入 `reality` 对象：`private_key`、`public_key`（配对的 URL-safe 无填充 Base64 X25519 密钥）、`short_id`（偶数位十六进制）、`target`（host:port）。`server_name` 为客户端使用的 Reality SNI；`path` 必填。Reality 服务端不需要本地证书。当前 Xray 26.10.31 的 Xray 客户端链路已在 VPS 连续三轮通过；这不代表其他客户端内核自动兼容。

Mihomo 官方文档明确不承诺与 Xray 26.7.11+ Reality 的兼容，因此当前基线拒绝/过滤该组合，而不是伪装成受支持节点。[上游 TLS/Reality 说明](https://wiki.metacubex.one/config/proxies/tls/)

CDN/反向代理需要额外验证 TLS 终止、域名、路径和端口所有权。候选生成不安装反向代理、不接管 80/443、不自动配置 CDN；高端口继续遵守当前项目 10000–65535 范围。

## 验证状态与复现

```bash
SBCTL_XRAY_BIN=/path/to/authenticated/xray cargo test --features test-signing --test xray_core -- --ignored
SBCTL_XRAY_BIN=/path/to/authenticated/xray cargo test --features test-signing --test preset_connections -- --ignored --exact tls_presets_carry_http_through_a_real_socks_client
# 单独选择外部 Reality 目标后执行（会产生外部 TLS 握手）：
SBCTL_XRAY_BIN=/path/to/authenticated/xray SBCTL_REALITY_TARGET=target.example:443 cargo test --features test-signing --test preset_connections -- --ignored --exact xhttp_reality_uses_external_decoy
```

第二项需要 curl，仅打开本地临时监听器并清理子进程，使用一次性叶证书指纹验证。为测试 HTTP 源站单独允许回环目标；正式生成配置不附带该例外。Xray 26.10.31 的 allowInsecure 已移除，测试与新生成器均不依赖它。

Windows/WSL 网络下载 Mihomo 工件曾返回 HTTP 504，之后从测试 VPS 下载官方 1.19.33 工件并校验发布摘要，已完成互通。早期本地自签 OpenSSL 目标原型未通过；改用显式外部目标进行握手，并将 Reality 请求超时预算从本地测试的 10 秒独立提高到 30 秒后，VPS 三轮转发通过。详见 [VPS 验收记录](vps-protocol-validation.md)。

依据：[Xray 固定版本传输配置](https://github.com/XTLS/Xray-core/blob/v26.10.31/infra/conf/transport_internet.go)、[Mihomo 传输文档](https://wiki.metacubex.one/config/proxies/transport/)、[sing-box 传输文档](https://sing-box.sagernet.org/configuration/shared/v2ray-transport/)。
