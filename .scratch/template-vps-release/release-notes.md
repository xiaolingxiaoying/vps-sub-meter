sbctl 0.0.11 按提供的 mihomo-test 模板生成完整订阅。

- Mihomo 仅生成“代理”和“自动选择”两个策略组；手动组依次列出全部节点、自动选择、DIRECT，Global 使用内核自带组。
- sing-box 标准完整订阅保留全部 DNS 服务器、SagerNet 规则源、门户直连顺序、TUN/mixed 设置及节点参数；保留 VMess Host 和 Hysteria2 上下行设置。
- 继续提供旧版客户端适配、minimal 规则、显式镜像、覆写及原有裸节点/URI 订阅。

升级已有覆写时，将旧引用 节点选择/direct/dns-direct/dns-proxy 对应更新为 代理/DIRECT/dns-cn/dns-remote，或移除已不需要的旧默认值覆写。完整模板说明见 src/subscription/templates/README.md。

发布制品由 GitHub Actions 构建，经过 Rust/Python 测试、真实内核和系统服务验收后生成签名 manifest。
