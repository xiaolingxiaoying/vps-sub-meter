# sbctl 审查修复与升级说明（2026-10-13）

本次实施覆盖 8 项审查问题：命名订阅凭据隔离、SMTP 权限、凭据失败回滚、
完整模式切换、轮换任务权限、卸载任务清理、总量修正显示及宽限期输入错误。
配置文件格式保持兼容，升级不轮换凭据、不重置流量账期。

当前状态：代码和回归测试已实施；真实三发行版 systemd 发布验收尚未通过，
因此这些问题尚未标记为完成全部发布验收。详见 ADR-0033。

## 使用变化

- 命名凭据打开订阅首页或二维码时，所有链接使用当前请求的凭据。
  新增或轮换命名凭据成功后，终端打印该命名凭据的新链接。
- 配置再生成、变更或升级会恢复邮件文件的 `root:root 0600` 权限，
  服务账号无法读取或删除邮件文件。管理员 override 的内容和权限保持不变。
- 已有轮换服务自动迁移为 root oneshot；定时器的启用状态及运行时间不变。
  未启用轮换的部署不会自动启用。两种卸载方式都清理受管邮件和轮换任务。
- 订阅总览页使用修正后的总量；`upload`/`download` 仍表示方向流量，
  订阅头 `total` 仍表示月额度。

## 模式切换

所有模式切换先校验、再更新工件和 systemd，健康检查通过后才报告成功。
失败时恢复旧部署；恢复失败会明确提示检查服务和配置。
默认使用 `/usr/local/bin/sing-box`，也可通过 `--sing-box-bin` 指定校验内核。

```sh
# Direct → 外部反向代理；外部代理负责公网 HTTPS
sbctl config switch-mode --mode external-proxy --listen-port 2080

# 域名模式 → IP fallback；域名证书降级为自签需要显式选择
sbctl config switch-mode --mode ip-fallback \
  --subscription-host 203.0.113.7 --http-port 2081 \
  --certificate-mode self-signed

# IP fallback → Direct；公网 80/443 必须空闲
sbctl config switch-mode --mode direct \
  --subscription-host sub.example.com --certbot-email admin@example.com

# 将协议 TLS 改为域名证书时，SNI 必须与订阅域名匹配
sbctl config switch-mode --mode direct \
  --certificate-mode domain --protocol-sni sub.example.com
```

未指定的通用设置保留原值。进入新模式必须提供该模式所需的主机和端口；
清除不适用的端口。`--certificate-mode` 选择协议证书，Direct 的订阅 HTTPS
始终需要有效域名证书。Direct 缺少证书时使用已保存的邮箱或 `--certbot-email`
完成 ACME，不隐式采用免邮箱注册。

External proxy 的域名协议证书必须已存在于该域名的 Certbot live 目录且有效。
切换会固定一份受管副本供内核读取，不修改外部代理证书权限。外部续期后重新
执行同模式 `switch-mode --mode external-proxy`，不指定新端口，即可刷新副本。

此前经命名首页或二维码取得的默认链接仍有效；本次修复不会自动撤销历史泄露。
需要撤销这些旧链接时，由管理员显式执行 `sbctl credential rotate` 并重新分发。

## 验证记录与复跑

- Windows：库测试、118 个 CLI 回归测试、Clippy 已通过。
- WSL Ubuntu 22.04：工作区测试、144 个 CLI 测试及 Clippy 已通过，
  包括 ACME 成功／失败模拟、六个跨模式方向、首次重启／健康检查失败及并发凭据操作。
- 真核矩阵：sing-box 1.10.7／1.11.15／1.12.25／1.13.21／1.14.1
  的客户端配置，以及 1.14.1 服务端配置和本机 Mihomo 加载检查已通过。
- 真实 Linux UID/GID：隔离挂载命名空间与 chroot 中验证了邮件不可读／不可删除、
  daemon 可写自己的状态、sing-box 可读配置和协议证书。
- 尚未验证：Debian 12、Ubuntu 22.04、Ubuntu 24.04 的完整 systemd 容器验收。
  本机 Docker Desktop 初始化 inference manager 失败，无法连接容器引擎。
  已扩展 `tests/acceptance/verify-real.sh` 覆盖真实轮换、迁移、权限和六方向切换。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --features sbctl/test-signing
cargo test -p sbctl --no-default-features --test release_trust

# 仅用于隔离权限测试；不修改宿主账号或宿主 /etc
sudo bash scripts/dev/check-storage-permissions.sh /absolute/path/to/linux/sbctl

# 引擎恢复后使用独立的生产和测试签名 Linux 构建运行既有三发行版验收
SBCTL_ARTIFACT=/absolute/path/to/production-sbctl \
SBCTL_TEST_ARTIFACT=/absolute/path/to/test-signing-sbctl \
sh tests/acceptance/run.sh
```

本次不发布版本，也不部署生产主机。
