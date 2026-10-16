# 内置客户端配置模板

来源：`C:\Users\ranly\Documents\mihomo-test` 的 `mihomo.yaml`、`sing-box.json`（2026-10-16）。Mihomo 保留原始 YAML；sing-box 仅移除 JSONC 注释后保存为标准 JSON。通过 `include_str!` 编译进程序，VPS 不依赖此 Windows 路径。

默认 `standard` 完整 sing-box 订阅以原模板为主体，保留全部五个 DNS 服务器、DNS 与路由规则顺序、SagerNet 规则源、TUN/mixed 入站、缓存和控制接口。仅填入真实节点、凭据、端口、TLS 信任策略，并应用明确配置的测速 URL、规则镜像、更新周期、DNS 模式、privacy 预设与覆写。VMess 保留实际 SNI 对应的 Host，Hysteria2 保留上行 50 / 下行 200 Mbps。

Mihomo 所有模板选项均采用原模板的两个策略组：

- `代理`：全部节点、`自动选择`、`DIRECT`，依次排列。
- `自动选择`：仅全部节点。

不生成自定义 `GLOBAL`、直连、故障转移或服务分类组，Global 由 Mihomo 内核提供。保留模板的 DNS（包括 `geosite:cn,private` 策略）、嗅探、TUN、门户直连及其优先级。组名与规则引用均无表情图标。

兼容性适配：sing-box 1.10/1.11 转换为旧版 DNS；1.10 用入站 sniff 与 DNS 出站；旧版移除不支持的 AnyTLS，缓存和 TUN stack 按版本控制。minimal 规则配置与 Mihomo 1.18 仍使用内置规则，不下载远程规则集或地理数据库。sing-box 的可选 global/split 策略目录仍保留。

裸节点 `sing-box.json`、URI 和服务端配置保留原有生成契约。完整 sing-box 组名现在为 `代理`、`自动选择`、`DIRECT`，DNS 标签与原模板一致。已有覆写若引用 `节点选择`、`direct`、`dns-direct` 或 `dns-proxy`，应分别改为 `代理`、`DIRECT`、`dns-cn`、`dns-remote`；不再需要的旧默认值覆写应移除，避免覆盖模板。

校园网示例网段 `202.202.145.0/24` 原样保留；需要其他网段时，通过覆写同时调整 TUN 排除与直连规则。
