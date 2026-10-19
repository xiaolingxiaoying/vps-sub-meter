# sbctl v0.0.14

修复订阅二维码和旧版客户端兼容问题。详细的逐条验证见 [订阅审查报告](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/sbctl-v0.0.14/docs/subscription-audit-2026-10-19.md)。

- 终端订阅和节点二维码增加四模块白色留边，每行换行前恢复终端颜色。
- `clash-1.18.yaml` 排除 mihomo 1.18 不支持的 AnyTLS，并同步过滤代理组引用，避免整份订阅加载失败。
- 仅启用 AnyTLS 时，CLI、菜单、邮件和总览页隐藏没有兼容节点的旧版订阅；请求这些订阅及其二维码返回 404。
- 通用节点 URI 正确编码密码中的特殊字符，覆盖 IPv4、IPv6 和中文密码；默认生成凭据的历史 URI 工件保持不变。
- 明确区分节点配置片段、完整订阅和客户端订阅扫码入口，避免“全版本兼容”和系统相机自动导入的误导。
- CI 分别使用现行 mihomo 和官方 1.18.10 检查对应格式，补足旧版兼容验证。

本地验证：467 项测试、fmt、clippy，以及 sing-box 1.10–1.14 和 mihomo 的真实内核矩阵通过。指定 VPS 上的 12 个订阅和二维码逐条验证；五种协议的实际代理请求均返回 HTTP 204。发布前隔离验证确认除旧版 Clash 修复外，其余 11 个订阅工件与现网逐字节一致，包括自定义分流规则。

升级后执行 `sudo sbctl regenerate`，再刷新客户端订阅；需要重新扫描终端二维码时使用 `sbctl qr --all` 或 `sbctl node --links --qr`。

支持 Debian 12、Ubuntu 22.04/24.04，提供 amd64/arm64 管理程序、sing-box 1.14.2 运行时、签名清单、安装脚本及 SHA256SUMS。

[安装说明](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/sbctl-v0.0.14/docs/installation.md) · [完整变更](https://github.com/xiaolingxiaoying/vps-sub-meter/compare/sbctl-v0.0.13...sbctl-v0.0.14)
