# sbctl（vps-sub-meter）第三轮项目审查报告

- **审查对象**：`vps-sub-meter` 工作树（2026-10-08），来源为 `singbox-sub-me` 的 `fix-ui` worktree 抽取的服务端副本
- **审查日期**：2026-10-08
- **审查方式**：只读代码审查（安装链路、更新与签名信任链、订阅服务、证书与权限模型、覆写 CLI、CI/验收/文档），关键结论逐条回到源码核对；网络可达的部分（GitHub Release API）做了实测
- **行号约定**：正文行号基于**审查时**的工作树；每条同时给出函数/符号名，修复后仍可按符号定位
- **本轮范围**：P0 七条已修复（见文末"本轮修复范围"）；P1/P2 只记录与建议，不在本轮实现

---

## 1. 总体结论

| 维度 | 评价 | 说明 |
|---|---|---|
| 安装事务设计 | 好 | 原子写、观察窗健康检查、ownership marker 作为提交点、`--replace-existing` 备份/恢复，方向正确 |
| 安装事务边界 | **差** | 事务开始点画错：证书写入与"删除管理二进制"都在事务语义之外（P0-2、P0-3） |
| 更新与回滚 | **差** | 回滚的权限映射与事务开始点不一致，回滚后系统状态比失败前更糟（P0-4） |
| 供应链 | 中 | 签名清单链路 fail-closed 设计扎实，但**文档指向的 Release 没有任何流水线生产它**（P0-1），且官方内核路径存在降级通道（P0-5） |
| 订阅服务 HTTP 边界 | 好 | 路径凭据 + 常量时间比较 + 统一 404 + 拒绝查询串 + 头部/连接上限 + 日志脱敏，未见越权面 |
| 文档与治理 | 差 | 安装文档与仓库实际能力矛盾，版本号/端口/命令描述漂移，无 LICENSE 正文 |
| 测试与 CI | 中 | 单测与 fixture 覆盖扎实；但**唯一能验证安装事务的 systemd 验收在本仓库从未跑过**（P2-19） |
| **综合** | **6.5 / 10** | 架构成熟度高于平均；问题集中在"事务边界"和"发布/文档治理"两块 |

**一句话结论**：这份代码的工程细节（原子写、常量时间比较、失败关闭的签名校验、systemd 加固）值得肯定，但它有两个系统性缺陷——**安装事务的边界画在了错误的时刻**（失败时会删掉用户的 CLI、留下让下次安装永久失败的状态、把回滚后的权限改坏），以及**文档承诺的发布能力在仓库里根本不存在**。这两类问题都不会被单元测试发现，只有真实 systemd 安装才会暴露。

---

## 2. P0：会让安装/更新不可用或损坏主机（本轮已修）

### P0-1 信任链指向一个没有发布流水线的仓库

**证据**

- 未提交改动把清单地址改成 `github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/...`：`src/update.rs`（`manifest_url_for_arch`）、`scripts/install.sh`（`default_manifest_url`）。
- `README.md` 与 `docs/installation.md` 的安装命令同样指向该地址。
- 但 `.github/workflows/` 只有 `ci.yml`：它只构建 artifact（`build-sbctl-linux-amd64`），不注入 `SBCTL_RELEASE_PUBLIC_KEY_HEX`、不签名、不生成 manifest、不生成 `install.sh`、不创建 Release。`release.yml` 按 `docs/repository-split.md` 有意未复制。
- `scripts/generate-manifest.sh` 需要人工提供 `SBCTL_SIGNING_KEY` 与 `SBCTL_SIGNER`，没有任何自动化调用者。

**影响**：文档给出的 `curl .../install.sh` 无来源；`sbctl update` 无来源；本仓库构建的二进制因为没有编译期公钥，`sbctl update` 一律以 `MissingTrustAnchor` 失败。这不是"配置遗漏"，而是**信任链断裂**。

**修复**：新增 `.github/workflows/sbctl-release.yml`（见"本轮修复范围"）。

### P0-2 安装失败回滚会删除 `/usr/local/bin/sbctl`

**证据**

- `src/lifecycle.rs::rollback_fresh_installation` 的删除列表含 `usr/local/bin/sbctl`。
- 唯一的豁免判据 `src/lifecycle.rs::predates_transaction` 只认 `etc/sbctl/config.toml` 与 `var/lib/sbctl*` 前缀。
- 该二进制由 `scripts/install.sh` 在事务开始前写入（`install -m 0755 … && mv -f`），且 `src/preflight.rs::existing_deployment_paths` 的冲突/归档列表**不含**它，所以 `--replace-existing` 也不会备份它。

**影响**：一次失败的安装会删除管理员唯一的 CLI。`ly` 变成悬空软链，`sbctl status/menu/uninstall` 全部不可用；Direct 模式下 certbot deploy hook（`src/lifecycle.rs` 中调用 `/usr/local/bin/sbctl certificate verify`）也一并失效，证书续期不再固定。

**修复**：`PreexistingState` 增加 `management_binary`，在事务开始前记录存在性，回滚时按"是否本次事务创建"决定是否删除。

### P0-3 证书目录在事务开始点之前落盘，失败后 preflight 永久拒绝重装

**证据**

- `src/cli/commands/install.rs` 的顺序是：`generated_artifacts_for_kernel()` → … → `check_sing_box_config()` → 之后才 `installation_started = true`。
- `generated_artifacts_for_kernel` 会调用 `src/subscription/render/singbox.rs::sing_box_server` → `certificate_tls_config` → SelfSigned 模式下 `ensure_self_signed_certificate`，在 `root == "/"` 时写到**绝对路径** `/var/lib/sbctl/certificates/<sni>/{cert.pem,key.pem}`。
- `src/preflight.rs::existing_deployment_paths` 明确把 `var/lib/sbctl/certificates` 列为 existing deployment。

**影响**：默认配置（SelfSigned + 五协议）下，只要 `sing-box check` 失败，就会留下证书目录；下一次安装直接被判为 `Existing deployment detected`，只能手工 `rm -rf` 才能重装。安装失败后"重试一次"这条最基本的恢复路径被堵死。

**修复**：把 `installation_started = true` 前移到事务捕获点之后、工件生成之前。

### P0-4 更新回滚用兜底属主/权限重写受管文件

**证据**

- `src/update.rs::restore` 对每条备份记录调用 `store.write_relative_locked`，最终进入 `src/config.rs::enforce_live_file_owner`。
- 该函数的路径→策略表只有三条（`etc/sing-box/config.json`、两个二进制），其余全部落到兜底 `sbctl:sbctl 0600`。
- 但回滚集合远不止这三类：`src/update.rs::MANAGED_PATHS` 含 5 个 systemd 单元与 certbot deploy hook；`rollback_paths` 在 Direct 模式追加 `var/lib/sbctl/certificates/<host>/{fullchain,privkey}.pem`。
- `restore` 只对 `usr/local/bin/` 前缀补 `set_executable`。

**影响**（回滚后系统比失败前更糟）：

1. **pinned 私钥变 `sbctl:sbctl 0600`**：`sing-box.service` 以 `User=sing-box` 读这份副本，改动后直接 `EACCES`；`grant_certificate_storage` 只 `chgrp` 不 `chmod`，不会自愈，此后每次重启数据面都失败。
2. **deploy hook 掉 `+x`**：`sbctl sing-box update` 的回滚只调 `restart_sing_box_service`，不做 unit/hook 协调，x 位永久丢失 → certbot 续期后 hook 持续失败，pinned 证书不再刷新。
3. systemd 单元被改成 `sbctl:sbctl 0600`：功能上 systemd 以 root 读取无碍，但把单元文件交给非 root 服务账号是防御纵深损失。

**修复**：把路径策略表补全（单元 `root:root 0644`、hook `root:root 0755`、pinned 证书 `root:sbctl-cert 0640` + 重新 `chgrp`），并让 `restore` 对 hook 恢复可执行位。

### P0-5 官方内核下载在摘要缺失时只警告，随后以 root 执行候选二进制

**证据**

- `src/update.rs::download_sing_box_official`：GitHub 未提供 `digest` 时只 `eprintln!` 警告并继续。
- 随后以 root 解压**整个归档**（`tar -xzf`，无成员路径校验、无 `--no-same-owner`），再以 root 执行候选二进制做"版本自检"（`confirm_sing_box_candidate` → `Command::new(candidate)`），最后复制到 `/usr/local/bin/sing-box`。
- 这条路径是 `sbctl install`（不给 `--manifest`）与 `sbctl sing-box update`（不给 `--manifest`）的默认分支，且都写在 `docs/installation.md` 里。

**影响**：供应链降级通道。能影响 API 响应使 `digest` 字段缺失的一方，可以在目标机以 root 执行任意二进制；"版本号自检"只校验 stdout，可伪造。

**修复**：摘要缺失一律 fail-closed（错误信息给出 `--manifest` / `--sing-box-bin` 两条替代路径）；解压改为"先列成员、拒绝危险名字、只解出唯一预期成员"，并加 `--no-same-owner --no-same-permissions`。

### P0-6 安装器先替换二进制、后询问"是否覆盖已有部署"

**证据**：`scripts/install.sh` 的顺序是 `mv -f /usr/local/bin/.sbctl.new /usr/local/bin/sbctl` → `ln -sf … ly` → 然后才用 `/usr/local/bin/sbctl install </dev/null` 做只读预检 → 用户选"保留并退出"时直接 `exit 0`。

**影响**：取消路径留下混合状态——磁盘上的 sbctl 已是新版，而 `config.toml`/工件/systemd 单元仍是旧版。`sbctl.service` 是 `Restart=on-failure`，之后任一重启就会用新二进制读旧配置；跨版本字段变化会进入 crash loop。升级场景（在已有部署上重跑安装脚本）尤其容易踩到。

**修复**：把"决策"整体前移到任何主机写入之前——候选二进制下载/校验到工作目录，预检用候选二进制，确认后才落盘。

### P0-7 `sbctl install` 非交互早退判据漏字段，带参数时静默空转

**证据**：`src/cli/commands/install.rs::install` 的早退条件只检查 `subscription_host`/`interface`/`reality_decoy_sni`/`sing_box_bin`/`manifest`/`guided` 六个字段。`--disable-protocol`、`--mode`、`--http-port`、`--proxy-host`、`--protocol-sni`、五个 `--*-port`、`--replace-existing`、`--no-start`、`--manage-firewall`、`--ipv4-only` 都不参与判断。

**影响**：`printf '' | sbctl install --disable-protocol vmess` 会打印 `install preflight passed` 并 **exit 0 什么都不做**；`--replace-existing` 这种破坏性意图同样静默空转。反过来，`scripts/install.sh` 又依赖"裸调用 + 非 TTY = 只读预检"这一契约，所以修复必须保留该语义。

**修复**：新增 `InstallOptions::is_bare()`，用解构模式强制覆盖全部字段（新增字段会成为编译错误），早退条件改为 `is_bare() && !stdin.is_terminal()`。

---

## 3. P1：功能缺陷与安全加固（本轮未实现）

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P1-8 | `src/cli/args.rs`（`SwitchMode`）、`src/cli/commands/config.rs`（`ConfigCommand::SwitchMode`）、`src/config.rs::validate_subscription_mode` | `config switch-mode` 双向都不可用：切 IpFallback 时 `http_port` 必为 `None`（校验要求必须有）；ip-fallback 切回时 `subscription_host` 是 IP（校验要求域名）。即使 direct ↔ external-proxy 成功，也只写 `config.toml`，不做 unit 收敛/重启 → 重启后 Direct 服务因缺 socket activation 进 crash loop，或 80/443 仍被残留的 `sbctl-http.socket` 占用 | 给 `switch-mode` 补 `--subscription-host`/`--http-port`，并复用 `lifecycle::restart_services`（它已正确处理 socket-activated 重启） |
| P1-9 | `src/cli/commands/serve.rs`、`src/subscription/serve.rs::serve` | Direct 模式静默忽略 `--bind`（构造出的 `0.0.0.0:0` 被丢弃）；ip-fallback 反而直接采用未校验的 `--bind`，可以与 `config.http_port` 和已打印的订阅 URL 不一致 | Direct 下拒绝 `--bind` 并说明；ip-fallback 下校验 bind 与配置一致 |
| P1-10 | `src/index_page.rs:171` | 总览页用 `traffic.received + traffic.transmitted`，忽略 `total_adjustment`；`TrafficReport::total()`（`src/traffic.rs`）走 `corrected_total`。`sbctl traffic set-used` 之后总览页与 `status --json`、`subscription-userinfo` 三个出口互相矛盾 | 改用 `traffic.total()` |
| P1-11 | `src/release.rs`（`ReleaseManifest`）、`src/update.rs::apply` | manifest 无 `serial`/`issued_at`/`expires_at`，`apply` 不与已安装版本比较 → 已公开发布过的旧签名 manifest 可被重放，把 sbctl + sing-box 一起降到有已知漏洞的版本 | 加序列号/有效期并在 apply 时拒绝降级 |
| P1-12 | `src/subscription/render/singbox.rs::ensure_self_signed_certificate` / `write_private_file` | 私钥复用只判 `is_file()`（跟随符号链接）、不校验 cert/key 配对；写入用 `write(true).create(true).truncate(true)`，无 `create_new`/`O_NOFOLLOW`。前提成立：`src/lifecycle.rs::prepare_daemon_storage` 把整棵 `/var/lib/sbctl` `chown -R sbctl:sbctl`，非 root 的 `sbctl` 账号可预置符号链接，之后 root 的 regenerate 会跟随写出（固定内容的任意路径 root 写原语） | 照抄 `src/certificate.rs::atomic_write_certificate` 的写法（`create_new` + 0600 + rename），并校验 cert/key 配对 |
| P1-13 | `src/cli/commands/system.rs::parse_grace` / `mask` | `parse_grace` 对 `--grace ""` 发生 `len()-1` 下溢，对 `--grace 30分` 落在字符中间 → `byte index is not a char boundary` panic；`mask` 的 `&credential[..min(6)]` 在配置损坏时同样会 panic | 用 `char_indices`/`strip_suffix` 解析单位，长度判断改 `get(..n)` |
| P1-14 | `src/subscription/serve.rs`（`IpBudget::admit`、`subscription_http_response`） | 桶数超过 4096 直接 `clear()` → 伪造大量源 IP 即可清空全局限流表；ExternalProxy 完全跳过限流（注释解释了原因，但文档没把"限流交给前置反代"写成运维要求） | 用 LRU/分片淘汰代替 `clear()`；把 ExternalProxy 的限流边界写进文档 |
| P1-15 | `src/update.rs::install_candidate_sing_box_locked` / `apply_sing_box` | 候选二进制"读后校验、按路径再执行"的 TOCTOU：`check_sing_box_config(candidate, …)` 用的是路径而不是已校验缓冲。默认下载路径不可触发，`--sing-box-artifact` 指向他人可写路径时可触发 | 统一改为"读一次、校验内存字节、写入用同一缓冲"（`apply` 已是正确写法） |

---

## 4. P2：文档、治理与测试覆盖（本轮未实现）

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P2-16 | `docs/installation.md:48`、`docs/subscription-guide.md:121`、`CONTEXT.md` | 端口写"必须大于 1024"，实际是 10000–65535；文档说 `sbctl restart` 会重新生成订阅，实际 `restart` 只 check + 重启（生成入口是 `regenerate`）；`CONTEXT.md` 仍是含客户端栈的 monorepo 术语表 | 逐条对齐（本轮已修前两条） |
| P2-17 | `Cargo.toml`、`docs/*`、仓库根 | 版本号自相矛盾（根包 `0.0.4` vs 文档多处 `0.2.0`）；`Cargo.toml` 声明 `MIT OR Apache-2.0` 但仓库无 LICENSE 正文 | 由 workspace version 驱动文档；补 LICENSE |
| P2-18 | `.github/workflows/*.yml` | 第三方 action 未 pin 到 commit SHA（`actions/checkout@v4`、`dtolnay/rust-toolchain@stable`、`actions/upload-artifact@v4`、`actions/download-artifact@v4`）。本轮新增的 `release.yml` 沿用同样的浮动 tag | 全部 pin 到 SHA；`release.yml` 与 `ci.yml` 一起改 |
| P2-19 | `tests/acceptance/run.sh`、`.github/workflows/sbctl-branch-ci.yml` | 验收套件硬要求 `SBTUI_ARTIFACT`/`SBCLI_ARTIFACT`，而本仓库没有这两个二进制，CI 里 `server-acceptance` job 被整段注释掉 → **本仓库没有任何会跑 systemd/Docker 安装验收的门**，而 P0-2/P0-3/P0-5 全部只有真机安装才暴露 | 让两条客户端腿可选，并恢复 CI job（本轮已修） |
| P2-20 | `Cargo.toml`、`src/release.rs::trusted_public_key` | `test-signing` feature 会让二进制信任公开的开发公钥；本仓库没有 release 门禁兜底，误构建即可发布 | 加发布门禁（本轮已加 `scripts/dev/release-trust-anchor-check.sh`） |

---

## 5. 值得肯定的设计

1. **签名清单 fail-closed**：编译期 `option_env!` 锚点、硬拒公开开发密钥、签名覆盖除 `signature` 外全部字段、canonical JSON 自排序、**先验签再信任 URL/摘要**；`install.sh` 用等价的 `jq -S -c 'del(.signature)'` + `openssl pkeyutl` 复现同一策略。
2. **原子写与提交点**：单文件写入是 `0600 临时文件 + write_all + sync_all + rename`；ownership marker 只在三次 1.1s 采样的观察窗通过后才写，能抓住 `Type=simple` 的秒级崩溃。
3. **HTTP 订阅端点基线**：路径凭据 + 常量时间比较（长度无关循环）+ 统一 404 + 拒绝查询串 + 16 KiB 头 / 5s 头超时 / 30s 连接上限 / 32 并发 / 令牌桶 + 日志脱敏 + 有意不信任 `X-Forwarded-For`。
4. **systemd 最小权限**：服务以专用 `sbctl`/`sing-box` 账号运行，`NoNewPrivileges`、`ProtectSystem=strict`、`ProtectHome`、`PrivateTmp`，Direct 模式用 socket activation 让 systemd 持有 80/443，非 root 进程拿到已绑定 fd。
5. **测试是真实黑盒**：限流不可区分性、凭据不泄漏、订阅读取不改 state、三发行版容器跑生产工件、`release_trust` 在 `--no-default-features` 下证明普通构建拒收开发密钥签名。
6. **凭据模型干净**：订阅凭据与协议凭据分离，命名凭据支持宽限期与轮换，`credential revoke` 用"过期时间点"而不是缺失字段表达吊销。
7. **覆写合并**：`reject_protected_server_inbounds` 在 `Overrides::load` 统一执行（包含未初始化部署），合并结果一律过真核 check；分层目录拒绝符号链接。

---

## 6. 本轮修复范围

| P0 | 修复内容 | 主要落点 |
|---|---|---|
| P0-1 | 新增签名发布流水线 + 运行时 pin + 发布门禁 | `.github/workflows/sbctl-release.yml`、`scripts/release-runtime-pins.txt`、`scripts/dev/release-trust-anchor-check.sh`、`scripts/generate-manifest.sh` |
| P0-2 | 回滚不再删除事务外写入的管理二进制 | `src/lifecycle.rs`、`src/cli/commands/install.rs` |
| P0-3 | 证书写入纳入安装事务 | `src/cli/commands/install.rs` |
| P0-4 | 回滚按路径恢复属主/权限/可执行位 | `src/config.rs`、`src/update.rs`、`src/lifecycle.rs` |
| P0-5 | 摘要缺失 fail-closed；只解出预期成员 | `src/update.rs` |
| P0-6 | 安装器先决策、后落盘 | `scripts/install.sh` |
| P0-7 | 早退判据覆盖全部参数 | `src/cli/args.rs`、`src/cli/commands/install.rs` |
| P2-19 | 客户端腿可选 + 恢复 CI 验收 job | `tests/acceptance/run.sh`、`.github/workflows/sbctl-branch-ci.yml` |
| P2-20 | 发布门禁拒绝开发密钥/无锚点构建 | `scripts/dev/release-trust-anchor-check.sh` |

验收与回归门见 `.scratch/p0-install-and-release-fixes/spec.md`。

---

## 7. 复核方式

```bash
# P0-2/P0-3：失败安装后管理二进制仍在、证书目录已回滚、且可立即重试
cargo test --features sbctl/test-signing --test cli -- a_failed_install
cargo test --features sbctl/test-signing --test cli -- install_arguments_that_carry_intent

# P0-4：回滚恢复 deploy hook 的可执行位
cargo test --lib -- update::tests::a_rollback_restores_the_certbot_hook_executable_bit

# P0-5：摘要缺失 fail-closed、归档成员校验
cargo test --lib -- update::tests::a_missing_upstream_digest_fails_closed_before_any_download
cargo test --lib -- update::tests::archive_member_names_that_tar_must_never_extract_as_root_are_rejected

# P0-1/P2-20：发布门禁（对带锚点的生产构建应通过，对 test-signing 构建应失败）
bash scripts/dev/release-trust-anchor-check.sh target/release/sbctl
python3 -m unittest discover -s scripts -p 'test_*.py'

# P2-19：服务端 L3 验收（无客户端二进制时自动跳过两条客户端腿）
sh tests/acceptance/run.sh
```
