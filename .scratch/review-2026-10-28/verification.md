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
