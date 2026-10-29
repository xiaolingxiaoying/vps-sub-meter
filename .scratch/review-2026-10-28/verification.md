# 2026-10-28 第二轮审查修复

## 范围

本轮由三个独立只读审查覆盖：订阅 HTTP/TLS/socket activation、安装与更新事务、流量/配置/向导。所有确认项已在工作树修复，并补了 Linux 回归；未把“无法复现的风格问题”计入。

## 已修复

| # | 严重度 | 缺陷 | 修复 | 回归 |
| --- | --- | --- | --- | --- |
| 1 | P1 | 仅注释/`null` 的 Clash YAML 覆写被当作 `null` 层，整份 Clash 工件被替换为 `null` | `load_yaml_layered` 跳过 `Null` 文档 | `a_comments_only_yaml_override_is_ignored_instead_of_replacing_artifacts` |
| 2 | P1 | 大写订阅域名通过校验，但 Certbot 目录为小写，导致签发后读不到证书 | 构造、`apply_options`、`load` 与证书路径统一小写 | `hostnames_are_lowercased_for_certificate_paths_and_load` |
| 3 | P2 | 向导把已保存的精确字节限额渲染为 `0.00 GiB` 后回读，静默清零/取整 | 新增 `ask_traffic_limit`，空输入保留原 `u64` | `traffic_topic_keeps_an_exact_byte_limit_on_empty_input` |
| 4 | P2 | 流量修正先算账期、后取锁；跨账期时给旧周期写入会被重置丢弃的修正 | 取锁后只采样一次 `now`，用同一时刻算账期和修正 | 全量流量测试 |
| 5 | P2 | 端口预检只探测 IPv6 TCP，未探测 Hysteria2/TUIC 的 UDP6 | `ensure_protocol_port_available` 增加 `[::]:port` UDP 绑定 | 平台端口测试 |
| 6 | P2 | `config.toml` 接受 `http://` 的额外编译规则集，绕过 CLI 的 https 要求 | 配置校验只接受 `https://` | `a_hand_edited_extra_rule_set_must_not_ship_plain_http_to_clients` |
| 7 | P2 | 两个 `sbctl install` 并发时，失败方按过期快照回滚删除成功方的部署 | 产物生成/配置提交持操作锁；提交前检查配置不存在；失败方检测到他人配置后跳过回滚 | `a_concurrent_installation_is_not_rolled_back_by_the_loser` |
| 8 | P2 | 更新回滚把证书对当两个独立文件备份/写回，可能混合代或只写一半 | 备份用 `read_pair` 一次读取；恢复用 `write_private_pair` 原子发布，保留辅助文件 | `a_rollback_publishes_the_pinned_certificate_pair_as_one_generation` |
| 9 | P2 | `curl | bash -s -- --guided` 把脚本管道当向导输入，替换二进制后才失败 | 先解析终端；`--guided` 转发时重定向已解析的输入 | `bash -n` + 安装器流程审查 |
| 10 | P2 | `accept` 的 `ECONNABORTED` 等瞬态错误被当作致命，单次 RST 可同时停掉 80/443 | 瞬态错误重试；EMFILE/ENFILE 短暂退避 | `transient_accept_errors_are_retried_instead_of_stopping_the_service` |
| 11 | P3 | 同一 Direct 端口的多个 systemd 监听器被静默覆盖 | 明确返回 `DuplicateDirectListener` 错误 | 编译与错误路径审查 |
| 12 | P3 | 官方版本 `v1.14.2` 进入资产名后找不到发布资产 | 解析后统一为 `1.14.2` 再构造 URL/成员/自检 | `official_versions_are_normalized_before_becoming_assets_or_members` |
| 13 | P3 | `parse_traffic_amount` 接受恰好 2^64 字节并饱和为 `u64::MAX` | 边界改为 `>= 2^64` 拒绝 | `rejects_invalid_human_traffic_amounts` |
| 14 | — | 证书刷新确认按路径清除新代标记（上一轮工单 05） | 代际绑定 + 与发布共用锁 | 见 `review-certificate-pair/verification.md` |

## 验证

- WSL Ubuntu 22.04：`cargo fmt --check`、`clippy --workspace --all-targets --all-features -D warnings` 通过。
- 全量：327 项库测试、169 项 CLI、13 项主程序单测、10 项合并库、4 项版本注册表通过；真核门禁按设计 `ignored`。
- 生产信任门禁 `cargo test -p sbctl --no-default-features --test release_trust` 通过。
- 后续本地 release 构建与 VPS 验收见本轮最后记录。

## 未关闭

- 观测夹具偶发失败：本轮 40/40 通过，仍未复现，保持 `needs-triage`。
- 安装 `--replace-existing` 与并发安装重叠时的旧部署备份恢复仍有理论窗口；本轮只保证两个全新安装不会互相删除。未把该边界声称为已解决。


## VPS 验收（本地构建，无 Actions）

最终 amd64 候选摘要 `f170a3f942d874b5b851df2f27af28ebeeef3309a60966aa6b5b410f79f12891`，上传到 `/root/sbctl-local-f170a3f942d8/`。测试前以 root-only tar 备份原部署（摘要 `646e7badef6e969e0ebf576a4ea73dde1e8fa9cafd4dba3aa46c50c909d15bc8`），并启动 15 分钟 systemd 恢复 guard。

- 隔离：向导/凭据冲突、fake-IP 规则、SRS/MRS 隔离、非法 interval、生产配置副本重新生成逐字节一致。
- 新修复实测：`config init` 接受大写 `Sub.Example.COM` 后落盘为小写；仅在 `overrides/clash-override.yaml` 写注释后 `regenerate`，Clash 工件仍含 `proxies:` 而非 `null`。
- 证书：第二文件写失败保留旧代、真实 sing-box 1.14.2 新代证书握手、失败刷新标记及同证书重试通过。
- 实际部署：候选替换后 `regenerate` 未改变配置/内核/工件/服务 PID；三项服务 active。
- 外部：正常 TLS 校验下 12 种订阅体与 VPS 工件逐字节一致、流量头存在、12 个二维码实际解码；overview 200，非法路径 404。
- 客户端：WSL Ubuntu 22.04 的 sing-box 1.14.1 经 Windows 上的 SSH TCP 转发/UDP 中继，VLESS Reality、VMess WebSocket、Hysteria2、TUIC、AnyTLS 全部 HTTPS 204。
- 恢复：停止 guard，原 v0.0.21 程序与工件、属主/权限、三项服务与会计定时器全部恢复；guard inactive。


## 第三轮（同日第二轮审查代理）

| # | 缺陷 | 修复 | 回归 |
| --- | --- | --- | --- |
| 15 | Mihomo `.mrs` 一律声明 `behavior: domain`，IP-CIDR 集载入 0 条规则 | `ExtraRuleSet.behavior`（默认 domain，`--behavior ipcidr`）；IP 集追加 `no-resolve`；菜单对 `.mrs` 询问类型 | `an_ipcidr_mrs_set_is_declared_with_its_behavior_and_no_resolve`、扩展的 CLI 规则集测试 |
| 16 | 服务端内核为 1.10 时，full 工件仍用 `action: reject` 且缺 `block-out` | 一次解析 full profile，把 legacy 标记传入规则列表注入 | `a_full_profile_targeting_1_10_uses_legacy_block_rules` |
| 17 | Clash `src`/`no-resolve` 修饰符被当成代理组目标，合法覆写被拒 | 按规则类型取目标字段，跳过逻辑规则 | 扩展的 `reference_checks_...` |
| 18 | 切回 Direct 时只检查 pinned 存在，不重新 pin 更新的 live 证书 | 比较指纹，live 更新即 pin | `switching_back_to_direct_repins_a_renewed_live_certificate` |
| 19 | 配置向导修改 Direct 主机后不签发/pin 新证书，HTTPS 静默不可用 | 提交前 `ensure_direct_certificate`；主机变化时签发或给出可操作错误 | `a_config_change_pins_a_renewed_live_certificate_for_direct_mode` |
| 20 | 安装在提交配置后释放操作锁，另一个配置命令可在健康检查期间提交，随后被安装回滚删除 | 操作锁保持到 ownership marker；安装内改用 `*_locked` 写路径 | 全量安装/CLI 测试 |
| 21 | `switch-mode` 未归一化新主机大小写，证书路径与 Certbot 不一致 | `desired` 中统一小写 | `a_mode_switch_host_is_lowercased_...` |
| 22 | 模式切换时复用同一托管端口被自身监听误报 `already in use` | 仅当旧端口属于即将停止的旧模式时跳过探测 | `reusing_the_running_subscription_port_is_not_a_false_conflict` |
| 23 | `--replace-existing` 冲突端口检测漏掉 IP fallback 的 `http_port` | `install_ports` 取 `http_port.or(subscription_listen_port)` | `replacement_install_ports_include_the_ip_fallback_public_port` |
| 24 | `/memory` 第一个样本恒为 0，状态页永远显示 0 字节 | 同一 reader 读取两个 JSON 值取第二个，超时 4s | `the_memory_endpoint_skips_the_synthetic_zero_sample` |
| 25 | `sing-box api enable --port 0` 成功但端点不可达 | 配置校验拒绝端口 0 | `a_hand_edited_zero_observation_port_is_rejected` |
| 26 | 纯 IP 连接的 `host: ""` 让目标列空白 | 空 host 回退 `destinationIP` | 扩展的连接描述测试 |
| 27 | 命名凭据可命名为 `default`，revoke 永远拒绝，泄露链接无法吊销 | add 拒绝该名称；revoke 优先检查同名命名记录；索引页给出 `--name` 提示 | 扩展凭据测试 |
| 28 | 观测 IO 错误不总带 `api enable` 提示，端口复用测试偶发失败 | 所有传输错误统一带可操作提示 | 全量门禁 |
| 29 | Direct 部署仅存在自签协议证书而没有 pinned 对时，更新备份被误判为不完整 | 两文件都不存在视为无 pinned 对；仅部分存在才拒绝 | `a_direct_deployment_without_a_pinned_pair...` |


## 第三轮 VPS 验收（本地构建，无 Actions）

最终 amd64 候选摘要 `ef99ca0a18c382fc5924e09ffe8f8c4308f7e07c74a30a8d6c07ad4045fa8699`，上传到 `/root/sbctl-local-ef99ca0a18c3/`。测试前备份原部署（摘要 `4886bc9b7babefb0375348e28d2c269db5625eac707ea32bb5313b6c9e26c90b`），启动 15 分钟 systemd 恢复 guard。

- 隔离实测：大写主机落盘为小写；仅注释 Clash 覆写后工件有效；`rule-set add --behavior ipcidr` 产出 `behavior: ipcidr` 与 `RULE-SET,...,no-resolve` 且不进入 sing-box 工件；`sing-box api enable --port 0` 退出 2 且配置字节不变；`credential add --name default` 被拒绝。
- 证书：第二文件写失败保留旧代、真实 sing-box 1.14.2 新代握手、失败刷新标记与同证书重试通过；生产配置副本重新生成 12 个客户端工件逐字节一致。
- 实际部署：候选替换后 `regenerate` 未改变配置/内核/工件/PID；三项服务 active；`sing-box status` 的“内核实时内存”为 7,520,256 字节（旧逻辑会显示 0），`api status` 为 7,528,448 字节。
- 外部：12 种订阅体与 VPS 工件逐字节一致、流量头存在、12 个二维码实际解码；overview 200，非法路径 404。
- 客户端：WSL Ubuntu 22.04 的 sing-box 1.14.1 经 Windows SSH TCP 转发/UDP 中继，五协议全部 HTTPS 204。
- 恢复：停止 guard，原 v0.0.21 `abde4999…` 程序、工件、属主/权限、服务与会计定时器恢复；guard inactive。


## 第四轮（回归与安全复查）

| # | 缺陷 | 修复 | 回归 |
| --- | --- | --- | --- |
| 30 | 失败的模式切换回滚只恢复配置，不恢复证书存储属主；Direct→非 Direct 的 `chown root:sing-box` 后回滚，`sbctl` 不再能读 pinned 私钥，Direct HTTPS 静默全断 | 回滚后按**恢复后的**模式重跑 prerequisites/storage；`apply_and_activate_locked` 同样按恢复后的配置决定 | 全量门禁；live `chown` 路径无 fixture 覆盖，记录为平台行为 |
| 31 | 向导从 ExternalProxy/IpFallback 切到 Direct 时，在提交前跑 Certbot，但 80/443 尚未启动，ACME 必然失败 | 无现成 live/pinned 材料时明确提示改用 `sbctl config switch-mode --mode direct`；有材料则直接 pin | 全量门禁 |
| 32 | Mihomo 规则目标校验对含逗号的正则匹配（`DOMAIN-REGEX,...,REJECT`）误判 | 改为取最后一个非 `no-resolve`/`src` 字段；逻辑规则仍跳过 | 扩展的 `reference_checks_...` 正则用例 |
| 33 | 菜单新增的 `.mrs` behavior 提示用 `required_menu_text`，回车默认值被过滤而静默取消 | 改用 `read_menu_choice`，空回答取 domain | 菜单代码路径；无交互测试 |


## 第四轮 VPS 验收（本地构建，无 Actions）

最终 amd64 候选摘要 `b5be40581b3a80e7e432f773a61689ad92da40bff762d329d59d09d43b8aa91b`，上传到 `/root/sbctl-local-b5be40581b3a/`。测试前备份摘要 `34fbdf82335356d1ad655f30a06a0728e00edd6c4904cc02a70b10ec675bbfaf`，启动 15 分钟 guard。

- 隔离：生产配置副本重生成 12 个客户端工件逐字节一致；大写主机、注释覆写、MRS `ipcidr`、`--port 0` 拒绝、`default` 名称拒绝全部通过。
- 新增验证：Clash 覆写 `DOMAIN-REGEX,^ads?\.[a-z]{2,3}$,REJECT` 经 `regenerate` 接受并出现在工件中。
- 证书：写失败保留旧代、真实核心新代握手、刷新失败重试通过。
- 实际部署：候选启动后配置/内核/工件/PID 不变；实时内存 `7,553,024` / `7,561,216` 字节（非 0）。
- 外部：12 种订阅逐字节一致、流量头、12 个二维码、非法路径 404 通过。
- 客户端：WSL sing-box 1.14.1 五协议全部 HTTPS 204。
- 恢复：原 `abde4999…` 程序、工件、服务、定时器恢复；guard inactive。


## 真实客户端内核矩阵

WSL 本地 `scripts/dev/wsl-real-cores.sh` 在当前树上通过：

- sing-box 1.10.7 / 1.11.15 / 1.12.25 / 1.13.21 / 1.14.1 对 standard/global/split/compact × standard/minimal/fake-ip/example 生成的 profile 全部 `check` 通过。
- mihomo 1.19.x（`~/bin/mihomo`）接受 standard/global/split/compact × standard/minimal × 两个 Clash 工件，以及真实 `.mrs` provider 二进制规则加载。


## 第五轮（服务端 1.10 兼容）

| # | 缺陷 | 修复 | 回归 |
| --- | --- | --- | --- |
| 34 | 服务端内核为 1.10 且主机有 IPv6 路由时，`config init --sing-box-bin 1.10` 生成的 server 配置使用 `route.rules[].action`/`dns.strategy`，真实 1.10 报 unknown field，安装/生成直接失败 | `sing_box_server` 接收 full profile 的 legacy 标记；1.10 用入站 `domain_strategy: ipv4_only`，不写 `dns.strategy`/route action | `a_legacy_server_config_pins_ipv4_on_the_inbound`；真实 1.10.7 `check` |


## 第五轮 VPS 验收（本地构建，无 Actions）

最终 amd64 候选摘要 `528669a0ca6d671c7501be2ff400579f1f982df4e2a1722eb0306541e1d5a063`，上传到 `/root/sbctl-local-528669a0ca6d/`。测试前备份摘要 `aa2596e053f1828166d7795a8d7308585b67eb2283cc0858dd8beb45cb0b4929`。

- 真实 1.10.7：`config init --sing-box-bin .../sing-box-1.10.7` 成功；server 配置使用入站 `domain_strategy`，`route.rules` 不再含 `action`；full 工件 `sing-box check` 通过，reject 规则使用 `block-out`。
- VPS 隔离：生产配置副本 12 个客户端工件逐字节一致；证书写失败/真实核心刷新通过。
- 实际部署：候选启动后配置/内核/工件/PID 不变；实时内存 7,561,216 字节。
- 外部：12 种订阅逐字节一致、流量头、12 个二维码、非法路径 404。
- 客户端：WSL sing-box 1.14.1 五协议全部 204。
- 恢复：原 `abde4999…` 程序、工件、服务、定时器恢复；guard inactive。
