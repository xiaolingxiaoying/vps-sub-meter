# sbctl 安装、兼容性与菜单完善

实现用户 2026-10-09 的五项请求：管理程序先安装；菜单选择本仓库固定内核或官方指定版本；
修复域名 TLS 安装顺序和 SNI 失配；统一 Android Clash 测速组；菜单接入配置、日志、连接、
内核状态、网卡 RX/TX；新增 SMTP 状态、流量、订阅和账期刷新提醒。

验证：Rust 单元和 CLI 回归、Linux clippy、真实 sing-box/Mihomo 配置检查。
真实 VPS 公网证书、Android 设备测速和真实 SMTP 投递需部署凭据与设备进行验收。
详细行为与边界见 docs/menu-and-email.md 和 ADR-0032。
