# vps-sub-meter

`vps-sub-meter`（命令别名 `vsm` / `ly`）是单一管理员在 Debian/Ubuntu VPS 上运行 **Xray + sing-box 双内核 PQC 代理** 与 **私有 HTTPS/Argo 订阅服务** 的极简控制面。

## 核心领域概念

- **Config (`/etc/vps-sub-meter/config.toml`)**：人类可读的主配置，包含域名/IP、网卡、月流量与重置周期、5 主力节点开关/端口/SNI、以及可选 Cloudflare Argo 配置。
- **Secrets (`/etc/vps-sub-meter/secrets.json`)**：`0600 root:root` 独立密钥仓库，保存订阅 Token、UUID、密码、X25519、ML-DSA-65 与 ML-KEM-768 密钥，绝不写入客户端订阅批次目录。
- **ServeAuth (`/var/lib/vps-sub-meter/serve-auth.json`)**：由 `vsm apply` 派生的只读运行时授权文件（仅含 Token 的 SHA-256 摘要，不含任何代理或 Argo 节点凭据），供非 root 守护进程 `vps-sub-meter.service` 使用。
- **ClientBundle / Generation**：每次 `vsm apply` 通过真实 `xray run -test` 与 `sing-box check` 校验后，原子发布的精简客户端工件（`clash.yaml`、`sing-box.json`、`shadowrocket.txt`、`uri.txt`；启用 Argo 时同步预编译至 `generations/<id>/argo/`）。
- **TrafficSnapshot**：严格按客户端视角映射（VPS `rx_bytes` = 客户端 `upload`，VPS `tx_bytes` = 客户端 `download`），结合 `boot_id` 防重启清零与双时区（`reset_tz` 计算账期，`display_tz` 展示）实时计算。
- **0 返回交互约束**：Bash 菜单 `scripts/ly.sh` 在确认前接受 `0` 或 EOF 并取消操作。Rust CLI 负责校验与写入，`config prepare` 在确认前只生成候选配置。
