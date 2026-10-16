# 内置客户端配置模板

来源：`C:\Users\ranly\Documents\mihomo-test` 的 `mihomo.yaml`、`sing-box.json`（2026-10-16）。Mihomo 保留原始 YAML；sing-box 移除 JSONC 注释后保存为标准 JSON。通过 `include_str!` 编译进程序，VPS 不依赖此 Windows 路径，也不会随来源文件的后续编辑自动更新。

生成器读取这里的客户端基础设置，并作以下适配：

- 节点、凭据、端口和证书策略由部署配置填入，模板中的 `YOUR-` 示例节点不会进入订阅。
- 保留现有策略组名称（`节点选择`、`自动选择`、`direct` 等）、standard/global/split 策略目录、自定义规则源、测速 URL 和覆写优先级。
- Mihomo 采用模板中的基础端口、DNS、嗅探和 TUN 设置；`geosite:` DNS 策略改为引用已生成的 `rule-set:`，避免额外下载 geosite 数据库。minimal / 1.18 配置改用内置域名列表，不引用远程规则集。
- sing-box 使用模板中的 TUN、mixed 入站、DNS 服务器、IPv4 优先和门户直连设置；1.10/1.11 继续转换为旧版 DNS，1.10 使用入站 sniff 和 DNS 出站，AnyTLS / store_dns 仍按版本控制。
- sing-box 的 DNS 标识保留 `dns-direct` / `dns-proxy`，国内解析的 privacy 预设仍切换为 DoH。未被策略目录使用的示例节点、冗余 DNS 服务器不输出。
- 模板中的校园网示例网段 `202.202.145.0/24` 原样保留在 TUN 排除与直连规则中。实际使用其他校园网段时，通过客户端覆写文件调整两处设置。

本次更换会更新完整客户端订阅的配置快照；裸节点 `sing-box.json`、URI 和服务端配置沿用原有生成契约。
