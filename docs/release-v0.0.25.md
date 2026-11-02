# sbctl v0.0.25

- sing-box 内核只从 SagerNet 官方发布包下载，移除本仓库来源选项和 Release 内核副本。签名清单继续固定官方版本与解压后摘要。
- 确认、协议启停、部署模式、证书模式等提示提供数字选项，兼容已有文字输入。
- VMess 等 TLS 协议说明证书自动生成/复用，SNI 支持 `1` 自动选择。
- 增加默认关闭的 Hy2 端口跳跃，包含范围/间隔设置、订阅适配、nftables 生命周期及失败回滚。
- 包含 dev 先前未推送的订阅加固与日常管理改进。

发布条件：fmt、Clippy、完整 Rust/CLI 测试、Python/JS 验证、真实客户端内核矩阵、隔离五协议数据面与 Hy2 跳跃验收，以及 GitHub CI 的 Debian/Ubuntu systemd 安装/更新/回滚门禁全部通过。

## 升级

从 0.0.24 或更早版本升级，请重新运行 README 的安装命令（下载最新 install.sh 再以 root 执行）。默认仅更新 sbctl，保留现有内核、服务、配置和订阅凭据。旧管理程序会把清单内核 URL 当成原始二进制，而本次清单指向官方归档，因此旧版 `sbctl update` 可能报告摘要不匹配；重新运行安装器后即可使用新的官方解压下载流程。

Hy2 跳跃入口：`sudo ly` → 部署与协议 → Hy2 端口跳跃。需安装 nftables，并在云安全组开放所选 UDP 范围。

源码标签：`sbctl-v0.0.25`，提交 `e916b7fad0e4862a40def7fd4c873f01c40094e6`。

- [dev CI](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/41394844915)
- [Release 构建](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/41396299306)

发布前已通过上述全部门禁，并独立核验两份清单的 Ed25519 生产签名、六个附件的摘要元数据和内核官方 URL；Release 不含 sing-box 内核副本。
