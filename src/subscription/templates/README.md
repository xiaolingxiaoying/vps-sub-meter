# 内置客户端配置模板

Mihomo 来源：`C:\Users\ranly\Documents\mihomo-test\mihomo-reviewed.template.yaml`（2026-10-17），原样保存为 `mihomo.yaml`。sing-box 的 `sing-box.json` 按同一策略适配。两份文件通过 `include_str!` 编译进程序，VPS 不依赖此 Windows 路径。

Mihomo 保留模板全部基础设置、DNS、嗅探、TUN 与规则顺序，包括四个 `review-*` MRS 规则集，不再使用旧版 GEOIP/geosite DNS 策略或校园网示例网段。`代理` 组按真实节点、`自动选择`、`DIRECT` 排列；自动组仅含真实节点。节点凭据、测速 URL、规则集镜像与更新周期仍来自部署配置。minimal 与 1.18 兼容订阅用内置域名/IP 列表替换远程规则集，同时移除 DNS 的规则集引用，避免启动时下载外部数据。

完整 sing-box 默认策略与新模板对应：私有域名及反向查询使用系统 DNS，门户、时间同步、STUN 和国内域名使用国内 DoH，其他查询经 `代理` 组访问境外 DoH。Fake-IP 使用 `198.18.0.0/16`、IPv4-only DNS，并保留私有域名、门户、时间同步及 STUN 例外。`services.googleapis.cn`、OpenAI/X 强制代理规则排在国内分流前；私有和国内域名/IP 分别直连。TUN 开启严格路由、MTU 1400，无示例网段排除；回环地址提供 mixed 7890、SOCKS 7891、HTTP 7892。手动组默认使用首个节点，与 Mihomo 的节点顺序一致。

sing-box 使用 SagerNet SRS 规则集；显式规则镜像仍覆盖默认 URL。其 DNS 规则一次选一个服务器，未把 Mihomo 的多个并发上游伪装成等价行为：默认国内使用 `223.5.5.5`，境外使用 `1.1.1.1`，保留 `dns-cn-doh`（doh.pub）供覆写选择。控制接口 `127.0.0.1:9090` 保留供客户端管理。1.11+ 嗅探限定 HTTP/TLS/QUIC 和模板端口、域名/IP 例外；不覆写目标。1.10 仅支持入站嗅探，无法完整表达这些例外，保留入站 sniff 且不启用目标覆写。1.10/1.11 转换为旧版 DNS；旧版移除不支持的 AnyTLS，缓存和 TUN stack 按版本控制。global/split 保留各自分流策略，同时采用新 DNS/TUN 基础设置。

裸节点 `sing-box.json`、URI 和服务端配置保留原有生成契约。管理员覆写最后应用；旧覆写仍可能覆盖新模板默认值。
