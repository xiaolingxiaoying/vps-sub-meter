修复单一来源可用未完成请求头或 TLS 握手占满订阅服务全部连接槽位的问题。HTTP、Direct HTTPS 与 ACME 在解析请求之前增加每来源 8 个活动连接额度，保留每监听器全局 32 个额度；连接结束、失败、超时或任务取消时自动回收。外部反向代理模式仍由前置代理按真实来源限流。

修复 JSON/YAML 覆写中错误类型的 `rules_mode` 被静默作为 `prepend` 接受的问题。显式值仅接受 `prepend`、`append`、`replace` 字符串，失败保持现有配置和订阅工件。

验证：Linux 格式检查、Clippy 和 487 项常规 Rust 测试；GitHub Actions 的 Debian 12 / Ubuntu 22.04 / Ubuntu 24.04 systemd 安装与失败恢复门禁、真实 sing-box / Mihomo 配置检查；Actions 生产二进制在授权 VPS 上完成连接隔离、真实服务及五协议 HTTPS 转发验证。VPS 已通过正式签名更新到 0.0.16，保留凭据、配置、流量状态及回滚备份。详见[完整验收记录](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/dev/.scratch/review-connection-isolation/verification.md)。

共享公网 IP 的客户端共享并发额度；此修复处理单来源连接占用，不保证抵挡分布式或链路带宽攻击。
