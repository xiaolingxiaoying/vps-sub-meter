# Acceptance fixture boundary

`verify.sh` is a black-box acceptance flow: it invokes only the administrator-visible
`sbctl` CLI and the subscription HTTP endpoint. `fixture.sh` supplies isolated host
state under `$work/<name>` for `/etc`, `/proc`, `/sys`, systemd command shims and
certificates. A fixture command is rooted below `usr/bin` and never falls back to the
container's real command.

The shell flow is suitable for local development and CI smoke checks. Production
support is established only by `run.sh` in a Debian/Ubuntu VM or equivalent real
systemd environment. WSL2 is a Development host for compilation, Rust tests and
simulated CLI checks; it is not evidence for the Production host release gate.

签名测试现在需要显式 `test-signing` 构建，通过 `SBCTL_TEST_ARTIFACT` 传入；生产工件仍通过 `SBCTL_ARTIFACT` 传入。两者不得混用，详见 [签名与验收隔离](../../docs/release-signing.md)。

`run.sh` 跑三发行版（debian:12-slim、ubuntu:22.04、ubuntu:24.04），每个容器依次执行
`verify-bootstrap.sh`（安装器：签名校验、参数转发、以及“已有部署时先决策后落盘”）、
`verify.sh`（fixture root 的事务、更新回滚点、卸载）与 `verify-real.sh`（真实 systemd 安装、
非 root 服务、socket activation、更新回滚后的权限与可执行位）。

最后执行 `verify-replacement.sh`：真实旧 sing-box 服务和占用 80 端口的 Nginx 的停止/禁用、
内核检查失败时恢复运行及启用状态、成功安装后保留 Nginx 配置。该脚本只用于可丢弃的 systemd 容器。

`SBCTUI_ARTIFACT`/`SBCLI_ARTIFACT` 是**可选**的：本仓库只有服务端 crate，未设置时脚本打印
`branch: server-only workspace - skipping the client legs` 并跳过 `verify-client.sh` 与
`verify-sbcli.sh`；设置后跑完整套件。CI 的 `server-acceptance` job 以无客户端变量的方式运行。

## 真实 Certbot／ACME 和代理数据传输

`run-acme.sh` 是独立的补充验收，使用 Debian／Ubuntu 仓库中的真实 Certbot、
真实 sing-box 工件和固定镜像摘要的 [Pebble](https://github.com/letsencrypt/pebble)。
每个发行版创建独立 Docker 网络、测试 CA 和 systemd 容器，并在结束后清理。
需要可运行 privileged/systemd 容器的 Docker 引擎；不会占用宿主公网 80/443。

```sh
SBCTL_ARTIFACT=/absolute/path/to/production-sbctl \
SING_BOX_ARTIFACT=/absolute/path/to/sing-box \
bash tests/acceptance/run-acme.sh
```

默认依次测试 Debian 12、Ubuntu 22.04、Ubuntu 24.04。
可使用 `ACME_BASE_IMAGES='debian:12-slim'` 单独复跑一种发行版。
生产 sbctl 工件不需要测试签名功能；sing-box 版本必须支持生成的五协议配置。

验收包括实际 HTTP-01 失败后的模式回滚、成功签发与受信任 HTTPS、AnyTLS
客户端到服务端的真实数据传输、强制续期与部署钩子、无进程重启的 HTTPS
证书更新、跨模式证书复用、首次 Direct 安装，以及卸载保留 Certbot 历史。
TLS 校验显式使用该测试 CA，未使用 `--insecure` 或关闭 ACME 挑战验证。
测试 CA 仅通过容器内 `REQUESTS_CA_BUNDLE` 和客户端 `--cacert`／`certificate_path`
使用，不加入宿主信任库。容器内关闭 Certbot 的续期随机等待，使验收时间可控。
首次安装后 HTTPS 监听器异步加载证书，探测使用有限次重试等待其就绪。

这些结果验证本地真实组件交互；公网 DNS、NAT／防火墙和 Let's Encrypt
生产服务仍需专用域名及公网测试主机另行验收。
