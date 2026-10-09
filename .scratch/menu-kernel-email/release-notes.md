## sbctl 0.0.7

- 默认安装器先安装 sbctl；在 `ly` 菜单选择 sing-box 内核来源。本仓库 Release 使用固定版本，官方来源支持指定稳定版本。
- 修复域名证书首次安装时的启动顺序，校验协议 SNI，并增加证书诊断、签发和续期菜单。
- 统一 Clash Meta/Mihomo 各代理组的测速 URL，补齐定时健康检查。
- 增加服务端自定义入站、出站和路由覆写，配置预览与私有文件导出，以及日志、内核状态、连接和 VPS 网卡 RX/TX 查看。
- 增加 SMTP 邮件配置、手动报告、每日状态和流量报告、账期刷新提前提醒；SMTP 强制使用 TLS，订阅链接默认不随邮件发送。

## 验证

通过 Rust 格式检查、Clippy、workspace 回归测试、真实 sing-box/Mihomo 配置检查及 Debian 12、Ubuntu 22.04、Ubuntu 24.04 的 Docker systemd 安装与回滚验收。正式工件由 Release 工作流生成，并通过生产信任锚与签名验证。

Android 实机测速、真实 SMTP 投递和公网 ACME 仍需在实际环境验收。

## 安装

下载本 Release 的 `install.sh` 并运行 `sudo bash install.sh`，随后使用 `sudo ly` 选择并安装内核。已有部署请先备份配置，再通过菜单更新管理程序和内核。
