# 验证结果（2026-10-09）

- WSL Ubuntu 22.04：cargo fmt、clippy --workspace --all-targets --all-features -D warnings、workspace test-signing 测试通过。
- Linux CLI：133 个测试通过，包含域名证书首次安装、SMTP 失败重试 / 去重、私有导出与卸载邮件备份 / 清理。
- 真核：sing-box 1.10.7、1.11.15、1.12.25、1.13.21、1.14.1 客户端配置矩阵通过；最新核心服务端配置通过。
- Mihomo：3 种模板 × 2 种规则模式 × 2 种 Clash 工件全部通过真实内核检查。
- Docker：Debian 12、Ubuntu 22.04、Ubuntu 24.04 的安装器验收通过，覆盖显式参数转发、已有部署取消重装、默认仅安装管理程序并保留内核。
- Python 安装器信任锚 / runtime pin 测试：5 个通过；Shell 语法检查与 git diff --check 通过。

测试使用隔离命令与 SMTP stub；没有真实邮件投递。公网 ACME、独立 VPS 与 Android 实机测速尚未验收。

## 0.0.7 发布前 Docker 完整验收

- 功能提交：79da2d1；生产与 test-signing 二进制分别构建并隔离使用。
- Debian 12、Ubuntu 22.04、Ubuntu 24.04 均通过 verify-bootstrap.sh、verify.sh、verify-real.sh。
- 覆盖真实 systemd、非 root 服务、订阅监听、卸载重装与注入更新失败后的恢复；测试容器已清理。
- 执行内容与 tests/acceptance/run.sh 服务端分支一致，通过 Windows Docker CLI 运行。
