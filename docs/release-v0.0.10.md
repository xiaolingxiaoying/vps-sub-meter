# sbctl v0.0.10

本版本将 mihomo-test 的 `mihomo.yaml` 和 `sing-box.json` 纳入内置客户端模板，改善 DNS 分流、校园网认证和 TUN 配置。

- Mihomo 采用模板中的基础端口、DNS、嗅探和 TUN 设置；sing-box 采用模板中的 TUN、mixed 入站、DNS 服务器和 IPv4 优先设置。
- 门户检测域名使用本地 DNS 并直连，TUN MTU 调整为 1400。模板保留示例校园网段 `202.202.145.0/24`，其他网段需通过客户端覆写调整 TUN 排除与直连规则。
- 节点和凭据继续动态生成，保留现有策略组名称、standard/global/split 模式、管理员覆写及 sing-box 1.10–1.14 版本兼容。
- Mihomo DNS 策略复用已生成的规则集，minimal 与 1.18 兼容配置使用内置域名列表，避免额外下载 geosite 数据库。
- 模板编译进程序，VPS 不依赖模板来源电脑上的路径。裸节点订阅、URI 和服务端配置沿用原有生成契约。

升级后重新生成订阅，客户端更新订阅即可获取新模板。模板适配说明见仓库 `src/subscription/templates/README.md`。

支持 Debian 12、Ubuntu 22.04/24.04，提供 amd64/arm64 管理程序、sing-box 运行时、签名清单、安装脚本及 SHA256SUMS。

[安装说明](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/sbctl-v0.0.10/docs/installation.md) · [完整变更](https://github.com/xiaolingxiaoying/vps-sub-meter/compare/sbctl-v0.0.9...sbctl-v0.0.10)
