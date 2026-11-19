# sbctl

`sbctl` 是 Debian/Ubuntu VPS 上的 sing-box 和 Xray 服务端管理工具。它支持 VLESS Reality、VMess WebSocket、Hysteria 2、TUIC、AnyTLS 等协议，以及 sing-box、Clash/Mihomo、Shadowrocket 和 URI 私有订阅。

## 安装与升级

在 Debian 12 或 Ubuntu 22.04 及以上版本的 VPS 上运行。支持 amd64 和 arm64，需要 systemd。用 SSH 登录后，以 `root` 执行下列命令。普通用户可先运行 `sudo -i`。

```bash
curl -fsSL https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh | bash
```

如果系统缺少 `curl` 或 CA 证书，先运行 `apt-get update && apt-get install -y curl ca-certificates`。也可以从 Release 下载脚本，检查后运行 `bash install.sh`。[下载 install.sh](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh)

安装器验证发布清单签名和管理程序的 SHA-256 摘要，VPS 无需安装 Rust。首次运行会安装 `sbctl` 和快捷命令 `ly`，并打开交互菜单。选择「首次安装」，按提示配置内核、协议和订阅。使用 Direct HTTPS 时，先将域名解析到 VPS 并放行公网 TCP 80/443，再按菜单提示放行协议端口。没有域名可选择 IP 模式。

已有 sbctl 部署可运行同一条命令升级管理程序，保留现有内核、配置、订阅凭据和流量账期。首次安装默认值、旧版恢复升级和 443 端口共用步骤见[安装与升级说明](docs/setup-and-upgrade.md)。

## 日常管理

| 命令 | 用途 |
| --- | --- |
| `ly` | 打开管理菜单，包括部署、协议、订阅和诊断 |
| `sbctl status` | 查看流量用量、剩余额度和重置时间 |
| `sbctl credential list` | 查看各设备的订阅凭据及宽限期 |
| `sbctl port status` | 查看监听端口和 TCP/UDP 归属 |

支持 Direct HTTPS、外部反向代理和 IP fallback 三种订阅模式。订阅链接使用路径凭据，支持 GET 和 HEAD，可以用 `curl -I` 检查响应头。

公网 HTTP、HTTPS 和 ACME 监听器各限制 32 个活动连接，同一来源最多 8 个。限制在握手和读取请求头前生效，超额连接直接关闭。共享公网 IP 的客户端共用来源额度。外部反向代理模式由前置代理按真实来源设限，后端保留全局限制。

- 统一节点、节点组、sing-box/Xray 双内核和旧配置迁移。[统一节点与双内核管理](docs/managed-deployment.md)
- 菜单项、对应的 CLI 命令、证书和诊断操作。[菜单与验证指南](docs/menu-validation.md)
- 私有订阅格式、客户端兼容性，以及 DNS、TUN 和分流覆写。[订阅指南](docs/subscription-guide.md) · [文件覆写与示例](docs/client-overrides.md) · [内置模板说明](src/subscription/templates/README.md)
- 在「部署与协议」中选择官方内核版本、生成或复用 TLS 证书，以及启用 Hy2 端口跳跃。[Hy2 端口跳跃说明](docs/hy2-port-hopping.md)
