# Menu actions and verification evidence

Run `sudo sbctl menu` (or `sudo ly`) to open the six main sections. The menu
uses the same handlers as the CLI. The CLI column below gives the equivalent
command where one exists; interactive prompts fill in values and confirmations.
An em dash means the operation is only exposed as a menu view or a scoped
configuration topic. Tests listed here are evidence to run, not a claim that
they passed in the current change. The normal automated suite uses temporary
deployment roots and fake system services where host interaction would
otherwise be required. Real-core checks are separate and need the pinned
upstream binaries described in the referenced test files. Systemd, ACME, network and privileged-port behavior use the disposable
Debian/Ubuntu containers in `tests/acceptance/run.sh`; WSL-only tests do not establish those properties.

## 新入口和首次配置

- 首页有六个任务分类；`0` 返回上级，首页 `0` 退出。订阅格式与二维码统一在「订阅与客户端 → 查看订阅链接 / 按格式显示二维码」。
- 「订阅与客户端 → 切换客户端模板」仅提供精简和标准，先说明差异并显示现有覆写，再预览和确认。精简保留审核版 DNS/TUN/分流，仅去掉自动测速组；标准保留原模板。引用检查或内核校验失败不发布新配置。
- 非交互也可运行 `sudo sbctl config template compact` 或 `sudo sbctl config template standard`；该操作不会重启 VPS 的代理服务，也不改流量账期。刷新客户端订阅后生效。
- 首次「部署与协议 → 快速安装」默认五协议；「引导安装」允许逐项配置协议和端口。两种路径分别询问是否定制模板、是否定制额度和账期，均默认跳过，可只接受其中一个。
- 跳过保留不限额、洛杉矶自然月刷新、上海显示；自定义锚定月仍输入首次完整日期时间。最后确认前显示摘要，取消不写入部署配置。非交互参数安装不增加等待输入。
- 旧 global/split 值不自动迁移；界面标为旧策略但不列为选择项，命令行仍兼容。文件覆写保留原有机制，日常改文件不必重新编译。

## 概览与诊断

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 完整部署状态 | `sbctl status` | `tests/cli/lifecycle.rs`: status output and redaction cases, using temporary roots. |
| 运行日志 | `sbctl logs` | Isolated systemd/journald fixture coverage; production journal access requires a systemd host. |
| 内核版本、进程和资源状态 | `sbctl sing-box status` and `sbctl sing-box version` | `tests/cli/observe.rs`: version/status and journal behavior use command fixtures. |
| 当前连接 | `sbctl sing-box connections` | `tests/cli/observe.rs`: API-off behavior; a running API check needs an isolated host with sing-box. |
| 证书状态 | `sbctl certificate status` | `tests/cli/certificate.rs`: validity, host coverage and redaction with certificate fixtures. |

## 部署与协议

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 快速安装 | `sbctl install` | `tests/cli/install.rs`: install flow, signed artifact and rollback fixtures; privileged service ownership needs an isolated systemd host. |
| 引导安装 | `sbctl install --guided` | `tests/cli/install.rs`: guided answers, cancellation and existing-deployment refusal using a temporary root. |
| 查看节点与监听端口 | `sbctl node` | `tests/cli/subscription_formats.rs` and lifecycle coverage: generated nodes without exposing credentials. |
| 协议启停、端口、SNI 和证书 | `sbctl config wizard` (interactive configuration) | `tests/cli/config_topics.rs`, `tests/cli/certificate.rs`, and `tests/cli/install.rs`; systemd, public ports and ACME need isolated host validation. |
| 节点分享链接 | `sbctl node --links` | `tests/cli/subscription_formats.rs`; the menu confirmation is covered by `tests/acceptance/verify-menu.py`. |

## 订阅与客户端

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 切换客户端模板（精简 / 标准） | `sbctl config template compact` or `sbctl config template standard` | `tests/cli/config_topics.rs` and `tests/acceptance/verify-menu.py`; `tests/version_profiles.rs -- --ignored` checks sing-box 1.10–1.14 cores, and `tests/clash_mihomo.rs -- --ignored` checks Mihomo. |
| 查看全部订阅链接 | `sbctl sub` | `tests/cli/subscription_formats.rs`; isolated HTTP route tests verify generated links and credential handling. |
| 按格式显示二维码 | `sbctl qr <format>` (or `sbctl qr --all`) | `tests/cli/subscription_formats.rs`: format matrix and QR generation. |
| 配置订阅入口与交付模式 | `sbctl config wizard` (subscription topic) | `tests/cli/config_topics.rs` and `tests/cli/lifecycle.rs`; Direct HTTPS socket ownership and external proxy routing require isolated host validation. |
| 查看命名订阅凭据 | `sbctl credential list` | `tests/cli/credentials.rs`: secrets remain masked. |
| 新增设备凭据 | `sbctl credential add --name <name>` | `tests/cli/credentials.rs`: independent named routes and rollback on failure. |
| 吊销指定凭据 | `sbctl credential revoke <name>` | `tests/cli/credentials.rs`: immediate and grace-window behavior. |
| 高级客户端设置 | 菜单内客户端配置主题 | `src/wizard.rs` 主题测试覆盖 DNS、规则来源、测速和 secret 脱敏；它保留现有配置事务。 |
| 轮换默认订阅凭据 | `sbctl credential rotate` | `tests/cli/lifecycle.rs` and `tests/cli/credentials.rs`: previous links stop working while proxy credentials remain unchanged. |

## 流量与通知

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 查看本周期流量 | `sbctl traffic show` | `tests/cli/traffic.rs`: accounting state with temporary roots and mocked counters. |
| 配置额度、账期、时区与网卡 | `sbctl config wizard` (traffic topic) | `tests/cli/config_topics.rs` and accounting tests; timezone and network-counter behavior are isolated from the host. |
| 修正本周期总流量 | `sbctl traffic set-used --bytes <total>` | `tests/cli/traffic.rs`: total-only corrections preserve RX/TX measurements. |
| 修正本周期 RX/TX | `sbctl traffic set-used --rx <rx> --tx <tx>` | `tests/cli/traffic.rs`: paired direction corrections and validation. |
| 建立 / 刷新账期 | `sbctl accounting-reset` (service operation) | Accounting reset tests exercise the persisted baseline in a fixture; a production timer check belongs on an isolated systemd host. |
| 配置邮件通知 | `sbctl email configure` | `tests/cli/management.rs` and email unit tests; validation uses temporary roots. |
| 查看邮件配置 | `sbctl email status` | `tests/cli/management.rs`: password redaction. |
| 立即发送邮件 | `sbctl email send` | Email unit/management tests; real delivery needs a controlled SMTP endpoint. |
| 启用定时邮件 | `sbctl email enable` | `tests/cli/management.rs`: timer file lifecycle in a temporary root; real activation needs isolated systemd validation. |
| 关闭定时邮件 | `sbctl email disable` | `tests/cli/management.rs`: timer removal in a temporary root; real activation needs isolated systemd validation. |

## 服务与更新

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 校验配置并重启服务 | `sbctl restart` | `tests/cli/lifecycle.rs` and configuration tests with a fake sing-box executable; actual service startup needs an isolated systemd host. |
| 启用本机连接观察 API | `sbctl sing-box api enable` | Lifecycle/config tests verify loopback binding and generated credentials; validate the running API on an isolated host. |
| 关闭连接观察 API | `sbctl sing-box api disable` | Lifecycle/config tests verify the endpoint is removed and the service transaction rolls back on failure. |
| 签发 / 修复 HTTPS 证书 | `sbctl certificate obtain` | `tests/cli/certificate.rs` uses a fake Certbot and certificates; public ACME challenge reachability must be checked on an isolated VPS. |
| 续期 HTTPS 证书 | `sbctl certificate renew` | `tests/cli/certificate.rs` covers renewal, pinning and rollback with fixtures; real Certbot/systemd reload is an isolated-host check. |
| 更新 sbctl | `sbctl update` | `tests/cli/update_release.rs` and `tests/release_trust.rs`: signature/hash verification and rollback fixtures. |
| 更新 / 切换 sing-box 内核 | `sbctl sing-box fetch --source <repository|official> [--version <version>]` | `tests/cli/update_release.rs`, `tests/version_profiles.rs -- --ignored`, and `docs/release-readiness-and-vps-test-plan.md`; kernel replacement and service health need isolated system validation. |

## 高级与维护

| Menu action | Underlying CLI | Automated / real-core / isolated-system evidence |
| --- | --- | --- |
| 编辑服务端 / sing-box / Mihomo 覆写 | `sbctl config override edit <server|sing-box|clash>` | Override tests cover merge, validation and rollback using temporary files; real sing-box and Mihomo parsing are covered by the ignored real-core suites. |
| 查看覆写文件与合并规则 | `sbctl config override show` | Configuration/override tests verify layer order and targets in temporary roots. |
| 校验全部覆写 | `sbctl config override validate` | Override tests plus `tests/version_profiles.rs -- --ignored`; Mihomo syntax requires `tests/clash_mihomo.rs -- --ignored`. |
| 预览配置 | `sbctl config preview --format <server|sing-box-full|clash>` | Configuration tests verify redaction and output; menu requires confirmation because output contains proxy credentials. |
| 导出配置 | `sbctl config export --format <format> --output <new-file>` | Configuration tests verify private permissions and no-overwrite behavior in a temporary directory. |
| 完整配置向导 | `sbctl config wizard` | `tests/cli/config_topics.rs` and wizard unit tests, using scripted prompts and temporary roots. |
| 重新生成并校验配置工件 | `sbctl regenerate` | Artifact/override tests use temporary roots and fake core checks; `tests/version_profiles.rs -- --ignored` and `tests/clash_mihomo.rs -- --ignored` validate real client-core compatibility. |
| 备份旧部署并重新安装 | `sbctl install --guided --replace-existing` | `tests/cli/install.rs`: replacement and recovery paths use system/service fixtures; real systemd recovery needs isolated host validation. |
| 卸载并保留备份 | `sbctl uninstall` | `tests/cli/lifecycle.rs`: managed units and binaries are removed while backup/data are retained. |
| 彻底清理 sbctl 数据 | `sbctl uninstall --purge` | `tests/cli/lifecycle.rs`: only sbctl-owned data is removed; manual deployments are protected. |
| 查看本地规则 | `sbctl rule list` | Rule CLI tests and subscription artifact tests use temporary deployment roots. |
| 添加本地规则 | `sbctl rule add <direct|proxy|reject|fakeip-filter> <entry>` | Rule tests verify validation and regenerated artifacts. |
| 移除本地规则 | `sbctl rule remove <direct|proxy|reject|fakeip-filter> <entry>` | Rule tests verify persistence and regeneration in a fixture. |
| 查看远程规则集 | `sbctl rule-set list` | Rule-set CLI tests verify listing and configured refresh values. |
| 注册远程规则集 | `sbctl rule-set add <name> --url <url> --outbound <target>` | Rule-set tests cover input validation and persistence; remote download behavior must use a controlled test endpoint. |
| 移除远程规则集 | `sbctl rule-set remove <name>` | Rule-set tests verify persistence and regenerated artifacts in a fixture. |
| 导入文本规则列表 | `sbctl rule-set import-list --url <url> --into <target>` | Rule-set tests should use a local HTTP fixture; no public CDN is needed. |
| 设置规则集刷新间隔 | `sbctl rule-set interval <duration>` | Rule-set tests verify duration parsing and regenerated artifacts. |

Menu numbering and dispatch are maintained from the shared catalog in
`src/cli/menu/navigation.rs`; its unit tests ensure every displayed nested
choice is selectable once and that installed deployments do not show fresh
install actions. The integration tests in `tests/acceptance/verify-menu.py` cover scripted
menu interaction. This guide records intended mappings and evidence locations;
it does not report test results or certify a VPS deployment.
