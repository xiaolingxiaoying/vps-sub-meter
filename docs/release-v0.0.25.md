# sbctl v0.0.25

- sing-box 内核只从 SagerNet 官方发布包下载，移除本仓库来源选项和 Release 内核副本。签名清单继续固定官方版本与解压后摘要。
- 确认、协议启停、部署模式、证书模式等提示提供数字选项，兼容已有文字输入。
- VMess 等 TLS 协议说明证书自动生成/复用，SNI 支持 `1` 自动选择。
- 增加默认关闭的 Hy2 端口跳跃，包含范围/间隔设置、订阅适配、nftables 生命周期及失败回滚。
- 包含 dev 先前未推送的订阅加固与日常管理改进。

发布条件：fmt、Clippy、完整 Rust/CLI 测试、Python/JS 验证、真实客户端内核矩阵、隔离五协议数据面与 Hy2 跳跃验收，以及 GitHub CI 的 Debian/Ubuntu systemd 安装/更新/回滚门禁全部通过。
