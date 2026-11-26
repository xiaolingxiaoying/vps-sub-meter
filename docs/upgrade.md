# 历史版本升级指南 (`scripts/upgrade.sh`)

`scripts/upgrade.sh` 与 `scripts/migrate_legacy.py` 为所有早期分支及历史版本提供**单一幂等升级入口**，无需先卸载或人工清理旧服务。

## 一、支持的历史版本与迁移内容

| 历史代际 | 识别依据 | 自动保留并迁移的状态 | 清理动作 |
| :--- | :--- | :--- | :--- |
| **Gen 1**（`sub-srv` + Caddy / `sing-box-yg`） | `/etc/sub-srv/config.conf` | `DOMAIN` 域名或 IP、`TOKEN`、`TRAFFIC_LIMIT_GIB`、重设周期与时区、`/var/lib/subsrv/tx_state.json` 已用流量、Caddy 已有域名证书 | 停用 `sub-server`、`caddy`、旧同步/重置 timer 等，自动切换至 5 节点 PQC 与内置 HTTPS |
| **Gen 2**（`sbctl` v0.0.1 ~ v0.0.28 及混合 PQC/Argo） | `/etc/sbctl/config.toml` | 域名/IP、端口与网卡、活跃订阅凭据（**自动剔除带 `revoked_at` 的已吊销凭据**）、原有节点 UUID/X25519/Password 与可选 ML-DSA-65/ML-KEM-768 密钥、Argo 域名/WS 入站/优选 IP、`boot_id` 流量账本与 Certbot 续签 `webroot_path` | 调用 `migrate-sbctl.sh` 停用遗留 `sbctl*` 单元与旧 Hy2 表（不改动宿主机其他防火墙表） |
| **Gen 3**（`vps-sub-meter` v2.0.0+） | `/etc/vps-sub-meter/config.toml` | 保留全部现有 `config.toml`、`secrets.json`、用户自定义 `templates/`、`argo-ips.txt` 与 `traffic-state.json` | 升级二进制与脚本后执行 `vsm apply` |

## 二、安全与回滚保障

1. **升级前全量加密权限备份**：自动打包现有 `/etc/vps-sub-meter`、`/etc/sbctl`、`/var/lib/sbctl`、`/etc/sub-srv`、`/var/lib/subsrv`、`/etc/letsencrypt/renewal` 等目录至 `/root/pre-vsm-upgrade-<UTC>.tar.gz`（权限 `0600`）。
2. **先校验后切换**：迁移生成 `/etc/vps-sub-meter/config.toml` 后先调用 `vsm status` 检验结构合法性，确认无误再停用旧服务。
3. **吊销凭据不复活**：仅导入活跃 Token，`sbctl` 历史上带 `revoked_at` 的已吊销设备凭据不会带入新版本。
4. **内核预检与事务回滚**：最终 `vsm apply` 通过 `xray run -test -c` 与 `sing-box check -c` 后才写入并重启数据面服务。

## 三、常用参数示例

```bash
# 1. 仅检测环境（只读，不改动系统）
sudo bash upgrade.sh --check

# 2. 使用已签名的官方 Release 在线升级
curl -fsSL https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/upgrade.sh | sudo bash

# 3. 无预编译 Release 时直接从指定分支源码构建升级
curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/dev/scripts/upgrade.sh | sudo bash -s -- --from-source --branch dev

# 4. 连带用仓库最新模板覆盖自定义模板（如需要同步更新 mihomo.yaml）
sudo bash /usr/local/lib/vps-sub-meter/upgrade.sh --force-templates
```

> 已安装的部署可直接执行 `sudo /usr/local/lib/vps-sub-meter/upgrade.sh`。升级会把 `ly` 切换到随包提供的 Bash 菜单，原来的 `vsm menu` 仍可打开同一菜单。
