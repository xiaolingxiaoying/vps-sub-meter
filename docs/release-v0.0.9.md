# sbctl v0.0.9

本版本支持备份旧部署、停止冲突服务后重新安装，解决卸载后残留 sing-box 服务文件导致安装预检拒绝继续的问题。

- 新增菜单入口：「安装与部署」→「3. 备份旧部署并重新安装（停止并禁用冲突服务）」。
- 支持 `sudo sbctl install --guided --replace-existing`，发布安装脚本也支持同时传入这两个参数。
- 替换安装备份并清理旧部署及失效启动链接，停止并禁用占用本次订阅/协议端口的系统级 systemd 服务；先停止相关 socket/timer，避免重新激活。
- 端口冲突的其他服务保留配置文件；安装失败恢复旧部署与服务状态，成功后保持冲突服务禁用。备份中的 `services.json` 记录原运行和启用状态。
- 增加发布安装器、残留服务恢复和真实 systemd/Nginx 替换安装验收，并纳入三发行版 CI。

使用普通升级可保留已有配置和凭据；选择替换安装会重建订阅和协议凭据，客户端需要更新。非系统级 systemd 管理的端口占用进程仍需自行停止。

支持 Debian 12、Ubuntu 22.04/24.04，提供 amd64/arm64 管理程序、sing-box 运行时、签名清单、安装脚本及 SHA256SUMS。

[安装说明](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/sbctl-v0.0.9/docs/installation.md) · [完整变更](https://github.com/xiaolingxiaoying/vps-sub-meter/compare/sbctl-v0.0.8...sbctl-v0.0.9)
