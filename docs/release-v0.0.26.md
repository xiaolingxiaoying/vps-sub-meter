# sbctl v0.0.26

- 统一管理 sing-box / Xray 节点与节点组，显示各客户端格式实际可用的节点数量。
- 简化首次安装，提供旧版恢复升级、协议补齐和按 SNI 共用 TCP 443 的命令与脚本。
- 支持带 SHA-256 校验的自定义内核下载，完善邮件设置与发送状态。
- 修复部署回滚、开机恢复、ACME 事务、Hy2 跳跃规则及客户端批次清理。
- 修复统一托管重启遗漏旧版凭据轮转服务迁移，保留定时器和订阅身份。
- 修复强制中断后遗留的原子写入临时文件在卸载清理和重装预检中被误判为已有部署的问题。
- 修复探测受 NO_PROXY 影响的问题，并在 Mihomo 模板中增加 Fake-IP 防漏规则。

## 升级

从本次 Release 下载并运行 `install.sh`，或使用 `sudo bash upgrade.sh`。
管理程序升级保留订阅地址、节点凭据、节点组和流量账期，不会自动补齐协议。
需要补齐时，先运行 `bash protocols.sh` 预览，再执行 `sudo bash protocols.sh --apply`。
安装与端口分流说明见 [安装、协议和旧版升级](setup-and-upgrade.md)。

## 发布验证

GitHub Actions 执行格式、Clippy、Rust/CLI、Python/JS、真实客户端内核矩阵及
Debian/Ubuntu systemd 安装、恢复和回滚验收。发布任务原生编译 Linux amd64 / arm64，
验证生产信任根，并为固定版本清单签名；所有门禁通过后生成 Release 附件。
sing-box 从 SagerNet 官方下载，本仓库不发布内核副本。

既有 VPS 候选验收记录见 [2026-11-17 部署记录](vps-deployment-2026-11-17.md)；
该记录对应发布前候选，不能代替本次 Actions 产物的实机验收。

## 已知限制

- sing-box 服务端出站覆写不覆盖 Xray 节点，Xray 防护仍有后续工单。
- 同一地址上的多个 UDP 443 服务不能通过 TCP SNI 分流共存。
- 自定义覆写和客户端全局模式可能影响 Fake-IP 防漏规则。
