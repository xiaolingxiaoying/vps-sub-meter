# sbctl

`sbctl` 是 Debian/Ubuntu VPS 上的 sing-box 服务端管理工具，支持 VLESS Reality、VMess WebSocket、Hysteria 2、TUIC 和 AnyTLS，并生成 sing-box、Clash/Mihomo、Shadowrocket 和 URI 私有订阅。

支持 Direct HTTPS、外部反向代理和 IP fallback 三种订阅模式，提供证书管理、流量统计、命名订阅凭据及签名更新。

## 安装

需要 Debian 12 或 Ubuntu 22.04 及以上版本、amd64/arm64 架构、systemd 和 root 权限。使用 Direct HTTPS 时，请先将域名解析到 VPS，并放行公网 TCP 80/443；协议端口按安装菜单提示放行。

```bash
sudo apt-get update && sudo apt-get install -y ca-certificates curl
curl -fL --retry 3 -o /tmp/sbctl-install.sh \
  https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh
test -s /tmp/sbctl-install.sh && sudo bash /tmp/sbctl-install.sh
```

已登录为 `root` 时去掉命令中的 `sudo`。

安装器会验证发布清单签名和文件摘要，并打开交互菜单选择内核与部署配置；VPS 无需安装 Rust。已有部署安装新版管理程序时，保留现有内核、服务、配置和订阅凭据。

修改客户端 DNS、TUN 或分流无需重编译，参见[文件覆写与现成示例](docs/client-overrides.md)。
