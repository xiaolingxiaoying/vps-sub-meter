# VPS 流量统计与订阅管理

## 快速开始

先运行 yonggekkk/sing-box-yg 脚本：
```bash
bash <(wget -qO- https://raw.githubusercontent.com/yonggekkk/sing-box-yg/main/sb.sh)
```

再运行本项目的统一安装器：
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/setup.sh)
```

## 其他功能

切换 IPv4/IPv6 出站策略：
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/switch_sb_mode.sh)
```

## 说明

- 所有云厂商（GCP / AWS / VMISS / 通用）使用同一个安装脚本，自动适配
- 支持三种订阅格式：Clash Meta (YAML)、sing-box (JSON)、Shadowrocket (TXT)
- 支持多种流量重置模式：自然月、指定日期循环、固定到期日
- 支持 RX + TX 双向流量统计
- 支持 BasicAuth + Token 免密两种访问方式
- 旧入口 URL（auto_setup.sh / gcp_sub_meter.sh / aws-sub-meter.sh / vmiss_sub_meter.sh）仍然可用
