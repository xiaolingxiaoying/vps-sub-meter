# VPS 节点订阅与流量统计

推荐使用一体化脚本；它会以固定版本安装/导入 sing-box-yg 核心节点，并配置 Clash、sing-box、Shadowrocket 订阅与实时流量头。

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/vps-sub-meter.sh)
```

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
