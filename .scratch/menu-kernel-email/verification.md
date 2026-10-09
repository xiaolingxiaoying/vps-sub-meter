# 验证结果（2026-10-09）

- WSL Ubuntu 22.04：cargo fmt、clippy --workspace --all-targets --all-features -D warnings、workspace test-signing 测试通过。
- Linux CLI：133 个测试通过，包含域名证书首次安装、SMTP 失败重试 / 去重、私有导出与卸载邮件备份 / 清理。
- 真核：sing-box 1.10.7、1.11.15、1.12.25、1.13.21、1.14.1 客户端配置矩阵通过；最新核心服务端配置通过。
- Mihomo：3 种模板 × 2 种规则模式 × 2 种 Clash 工件全部通过真实内核检查。
- Docker：Debian 12、Ubuntu 22.04、Ubuntu 24.04 的安装器验收通过，覆盖显式参数转发、已有部署取消重装、默认仅安装管理程序并保留内核。
- Python 安装器信任锚 / runtime pin 测试：5 个通过；Shell 语法检查与 git diff --check 通过。

测试使用隔离命令与 SMTP stub；没有真实邮件投递。公网 ACME、VPS 真实 systemd 运行和 Android 实机测速尚未验收。
