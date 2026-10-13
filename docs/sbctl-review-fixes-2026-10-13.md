# sbctl 审查修复与升级说明（2026-10-13）

本次实施覆盖 8 项审查问题：命名订阅凭据隔离、SMTP 权限、凭据失败回滚、
完整模式切换、轮换任务权限、卸载任务清理、总量修正显示及宽限期输入错误。
配置文件格式保持兼容，升级不轮换凭据、不重置流量账期。

当前状态：代码和回归测试已实施，既有 Debian 12、Ubuntu 22.04、Ubuntu 24.04
Docker/systemd 验收均已通过。详见 ADR-0033 和下方验证记录。

## 使用变化

- 命名凭据打开订阅首页或二维码时，所有链接使用当前请求的凭据。
  新增或轮换命名凭据成功后，终端打印该命名凭据的新链接。
- 配置再生成、变更或升级会恢复邮件文件的 `root:root 0600` 权限，
  服务账号无法读取或删除邮件文件。管理员 override 的内容和权限保持不变。
- 已有轮换服务自动迁移为 root oneshot；定时器的启用状态及运行时间不变。
  未启用轮换的部署不会自动启用。两种卸载方式都清理受管邮件和轮换任务。
- `credential revoke NAME` 同时撤销该名称下的当前链接和仍在宽限期内的历史链接。
  指定撤销宽限期时，不延长历史链接已有的到期时间。
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
  daemon 可写自己的状态、sing-box 可读配置和协议证书，以及 ACME 挑战目录可读。
- 2026-10-13 Docker 复跑：Debian 12、Ubuntu 22.04、Ubuntu 24.04 的安装器、
  模拟事务和真实 systemd 验收全部通过，完整脚本退出码为 0。
  覆盖真实轮换、旧单元迁移、邮件权限、六方向切换、HTTP/HTTPS 监听、
  HTTP-01 挑战响应、流量状态保留、失败恢复和两种卸载。
- 复跑中修复了两个 Linux 回归：`fs.protected_regular=2` 下 root 重新打开
  服务账号拥有的锁文件时避免 `O_CREAT`；逐路径授予 ACME 的 `.well-known`
  和 `acme-challenge` 目录访问权限。测试脚本也改为使用重新安装后的当前凭据。
- 既有 `run.sh` 容器验收使用本地证书和模拟 sing-box 进程。
  服务端仓库未提供 sbcli/sbtui 工件，其可选客户端验收分支按脚本约定跳过。
  本地完整日志：`.scratch/acceptance-bin/docker-acceptance-final.log`。
- 继续验证：新增 `run-acme.sh`，使用真实 sing-box 1.14.1、Pebble 2.10.1 测试 CA
  和发行版 Certbot（Debian 12：2.1.0，Ubuntu 22.04：1.21.0，Ubuntu 24.04：2.9.0）。
  三发行版均通过完整补充验收，固定脚本复跑退出码为 0。
  覆盖实际 HTTP-01 失败后的配置／工件恢复、成功签发及受信任 HTTPS、AnyTLS
  客户端到真实服务端的数据传输、强制续期部署钩子、HTTPS 证书热更新且进程不重启、
  跨模式证书复用、首次 Direct 安装和卸载保留签发历史。该轮本地验收未发现新的产品代码问题。
- 补充验收仅修改测试基础设施：使用测试 CA、关闭容器内续期随机等待、有限次重试
  等待首次安装的证书异步加载。未关闭 TLS 校验或 ACME 挑战验证。
  日志：`.scratch/acceptance-bin/docker-acme-debian-final.log`、
  `.scratch/acceptance-bin/docker-acme-ubuntu-final.log`。
  此轮仅覆盖本地网络，后续公网结果见下一条。
- 授权 VPS 的公网验证：Ubuntu 22.04、Certbot 1.21.0、既有 sing-box 1.14.2。
  升级前保存完整部署备份；sbctl 从 0.0.6 升级到本次 0.0.7 构建。
  升级期间暂停流量写入任务，核对配置、流量状态、内核文件 SHA-256 一致后恢复任务。
  Let's Encrypt staging 的实际 HTTP-01 与 `certbot renew --dry-run` 已通过，
  正式证书内容未变。旧 Certbot 不支持 dry-run 的 `--run-deploy-hooks`，因此另行执行
  受管钩子，验证固定证书副本与权限。生产 CA 强制重签未执行。
- 公网凭据验证发现并修复同名历史链接撤销遗漏：原实现只修改第一条有效记录，
  可能使宽限期旧链接在撤销后继续有效。新增回归覆盖同名有效历史、当前、已到期
  记录及其他名称；立即撤销全部有效同名记录，带宽限期撤销保留较早的原到期时间。
  修复后在 VPS 外部以正常 TLS 校验访问首页、订阅和二维码，并实际解码二维码，
  默认、命名、宽限期旧链接隔离及撤销后统一 404 均通过。临时测试凭据全部撤销，
  原有默认凭据、协议配置和服务保持正常。Windows 9 个凭据测试、Linux 269 个
  库测试和 145 个 CLI 测试、Clippy 已通过。
- 同名撤销补丁的独立生产／测试签名工件再次执行既有三发行版 Docker/systemd
  验收，Debian 12、Ubuntu 22.04、Ubuntu 24.04 全部通过，脚本退出码为 0。
  日志：`.scratch/acceptance-bin/docker-acceptance-public-revoke-fix.log`。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --features sbctl/test-signing
cargo test -p sbctl --no-default-features --test release_trust

# 仅用于隔离权限测试；不修改宿主账号或宿主 /etc
sudo bash scripts/dev/check-storage-permissions.sh /absolute/path/to/linux/sbctl

# 使用独立的生产和测试签名 Linux 构建运行既有三发行版验收
SBCTL_ARTIFACT=/absolute/path/to/production-sbctl \
SBCTL_TEST_ARTIFACT=/absolute/path/to/test-signing-sbctl \
sh tests/acceptance/run.sh

# 真实 Certbot、测试 ACME CA、sing-box 及代理数据传输的三发行版补充验收
SBCTL_ARTIFACT=/absolute/path/to/production-sbctl \
SING_BOX_ARTIFACT=/absolute/path/to/sing-box \
bash tests/acceptance/run-acme.sh
```

本次不发布版本，也不部署生产主机。
