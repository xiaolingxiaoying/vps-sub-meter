# Hy2 端口跳跃

运行 `sudo ly`，进入「部署与协议 → Hy2 端口跳跃」。先启用 Hysteria2，然后选择：

- `1` 启用、`2` 关闭、`0` 取消；默认关闭。
- 范围 `1` 为 20000–30000、`2` 为 40000–50000，也可输入 `起始-结束`。
- 间隔 `1` 为 30 秒、`2` 为 60 秒，也可输入 5–3600 秒。

端口范围须在 10000–65535 内，至少包含两个端口，不能覆盖其他协议、订阅或观测 API 的监听端口。原 Hy2 端口保持可用；关闭 Hy2 协议会同时关闭跳跃。仅更改其他配置会保留跳跃参数。

在 VPS 安装 `nftables`，并在云安全组放行整个 UDP 范围。宿主防火墙仍由管理员管理；UFW 的 INPUT 规则通常需放行原 Hy2 监听端口，页面也会打印范围放行命令。若使用外部端口映射，公网范围必须正确映射到 VPS。

sing-box 仍以非 root 运行。只有启用跳跃时，受管理的 systemd 单元才增加 root 的启动前和停止后 hook，在专用 `inet sbctl_hy2` 表内将流量转发到原 Hy2 端口；规则仅匹配配置网卡进入且目的地为本机的 UDP。该表名为 sbctl 保留，请勿手动使用。规则同时覆盖 IPv4/IPv6，不清空其他 nftables 表。停止、卸载、关闭跳跃会清理该表，重启恢复。缺少 nftables 或规则应用失败会使服务启动失败，并触发配置编辑的回滚。

客户端订阅：

- sing-box 1.11 及更新版本使用 `server_ports` 和 `hop_interval`；1.10 自动保留固定端口。
- Mihomo 使用 `ports` 和 `hop-interval`。
- URI、分享链接及 Shadowrocket 使用 Hy2 官方 URI 的多端口地址。URI 标准不包含间隔，导入后使用客户端默认间隔；需精确间隔请用 JSON/YAML 订阅。客户端需支持多端口 URI。

验证包括范围与冲突校验、取消/确认交互、真实 sing-box 版本矩阵，以及隔离网络命名空间内的真实 Hy2 流量、跨跳跃间隔传输与 nftables 清理。

官方依据：[sing-box Hy2](https://sing-box.sagernet.org/configuration/outbound/hysteria2/)、[Hysteria 端口跳跃](https://v2.hysteria.network/docs/advanced/Port-Hopping/)、[URI 规范](https://v2.hysteria.network/docs/developers/URI-Scheme/)、[Mihomo Hy2](https://wiki.metacubex.one/config/proxies/hysteria2/)。
