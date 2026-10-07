先运行yonggekkk/sing-box-yg脚本
```bash
bash <(wget -qO- https://raw.githubusercontent.com/yonggekkk/sing-box-yg/main/sb.sh)
```
再使用
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/auto_setup.sh)
```
切换ipv4或ipv6出站
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/switch_sb_mode.sh)
```
gcp
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/gcp_sub_meter.sh)
```

aws
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/aws-sub-meter.sh)
```

vmiss
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/vmiss_sub_meter.sh)
```

nexus
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/nexus-sub-meter.sh)
```

## sbctl 服务端管理工具

本分支保留原有 VPS 流量统计脚本，并新增独立的 sbctl 服务端管理工具，位于 sbctl/。它支持 Debian 12 / Ubuntu 22.04+、systemd、amd64 和 arm64。功能、部署边界和完整安装说明见 [sbctl/README.md](sbctl/README.md)。

### 安装 sbctl（GitHub Actions 预构建 Release）

支持 Debian 12 / Ubuntu 22.04+、systemd、amd64 和 arm64。安装器会下载与 VPS 架构匹配的预构建 `sbctl` 和 sing-box，验证签名与摘要，然后启动完整配置向导；VPS 不需要安装 Rust、下载源码或自行构建。

以有 `sudo` 权限的普通用户执行：

```bash
curl -fL --retry 3 -o /tmp/sbctl-install.sh \
  https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/install.sh
test -s /tmp/sbctl-install.sh && sudo bash /tmp/sbctl-install.sh
```

如果已经以 `root` 登录，将最后一行改为 `test -s /tmp/sbctl-install.sh && bash /tmp/sbctl-install.sh`。向导会写入 `/etc/sbctl` 配置并创建 systemd 服务；安装器默认不更改防火墙。首次安装前请确认主机上没有需要保留的 sing-box 部署。完整选项与安全边界见 [sbctl/README.md](sbctl/README.md)。