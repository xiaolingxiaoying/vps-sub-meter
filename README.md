# VPS 节点订阅与流量统计

当前分支 `codex/sing-box-integration` 提供一体化入口：固定版本安装或导入 sing-box-yg，管理五协议节点，并提供 Clash、sing-box、TXT（Shadowrocket）订阅与实时 `subscription-userinfo` 流量头。

## 一体化脚本（本分支）

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/codex/sing-box-integration/vps-sub-meter.sh)
```

主菜单包括：

- 全新部署或保留式导入现有 sing-box-yg。
- 五协议节点与三种订阅导出：`/etc/s-box/clmi.yaml`、`sbox.json`、`jhsub.txt`。
- 实时流量统计、订阅鉴权、周期基线与 `subscription-userinfo`。
- 域名 HTTPS 默认模式，以及明确警告的 IPv4/HTTP 模式。
- WARP 与出站管理：固定快照的标准 WARP、WARP-plus 本地 SOCKS5、多地区 Psiphon SOCKS5、IPv4/IPv6 与域名分流。

WARP 默认不启用。启用后仅代理在菜单中添加的域名后缀，未命中规则的订阅节点流量继续直连；流量头始终按 VPS 网卡字节实时统计。WARP-plus 仅监听 `127.0.0.1`，并通过 systemd 管理。

> 分支命令仅用于测试本分支内容；合并到 `main` 后，请改用 `main` 地址。

## 旧脚本入口

旧流程：先运行 yonggekkk/sing-box-yg 脚本
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
