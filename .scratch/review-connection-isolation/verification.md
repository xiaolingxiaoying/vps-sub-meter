# v0.0.16 连接隔离与覆写校验验收

日期：2026-10-20。生产代码及 tag 提交：`bd5677027c96e1a60611d9d8a545f1458f4e2a8f`。

## 构建来源

[Release Actions 39476665755](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39476665755) 的 `release-sbctl-linux-amd64` 工件；注入生产公钥，不启用 test-signing。本地没有构建用于 VPS 测试的生产二进制。

候选及 VPS 测试文件 SHA-256：`b431e914ed814a467d9d864527717d185d7eae89e8f0df4e55b93798491e94bc`。

## 旧版本复现与修复结果

VPS 上使用独立网络命名空间、临时配置和测试证书，测试监听器仅存在于该命名空间的 loopback。真实部署未参与连接占用实验。旧版使用已发布的 v0.0.15 资产，其 SHA-256 为 `48ced52eb14628ead81ceaa6424cc67b440935802b557f9f5f3b7154e449ee7e`。

| 场景 | v0.0.15 | Actions v0.0.16 |
| --- | --- | --- |
| IP fallback 未完成请求头 | 同来源保留 32 连接，另一来源失败 | 同来源保留 8 连接，另一来源正常 200 |
| Direct HTTPS 未完成握手 | 同来源保留 32 连接，另一来源失败 | 同来源保留 8 连接，另一来源正常 TLS/200 |
| ACME 未完成请求头 | 同来源保留 32 连接，另一来源失败 | 同来源保留 8 连接，另一来源可正常获得 404 |
| ExternalProxy | 保留全局 32 限制 | 保留全局 32 限制，不按代理地址收紧 |
| 连接结束 | 额度恢复 | 额度恢复 |
| JSON/YAML/server rules_mode 类型错误 | 静默接受 | 明确失败，配置和工件字节保持不变 |

复现脚本：[verify_isolated.py](verify_isolated.py)。测试中的 TLS 客户端信任临时证书 CA，未跳过证书校验。Rust 回归另验证任务取消后的额度回收、IPv4 映射地址共用额度和全局上限。

## 本地门禁

- fmt、所有目标/特性 Clippy 通过；487 项常规 Rust 测试通过。
- 普通构建 release_trust 1 项通过；安装脚本 Python 单测 5 项通过。
- 第一次 v0.0.16 完整运行有一个既有观测夹具偶发失败，随后该测试及全部测试复测通过；记录在 [issue 03](issues/03-observation-fixture-flake.md)，尚未声称找到或修复根因。同提交 Release Actions 的 ci/test 也通过。

## 真实 VPS 服务

复用已提交的 `.scratch/review-2026-10-20-followup/vps_live.py`，将 v015 路径和 unit 前缀替换为 v016，在 VPS 上暂存为 `/root/sbctl-v016-live.py`。开始前建立 root-only 归档，设置 15 分钟 systemd 自动恢复任务；测试不轮换真实订阅凭据。

- 实际部署载入 Actions 候选；12 种订阅和二维码、流量头、非法凭据及 query 路径符合预期。
- API 启用/停用后，订阅服务账号保持读取私钥权限，真实 HTTPS 正常。
- 服务端覆写编辑及 clear 均改变实际 sing-box PID；非法覆写被真内核拒绝且源文件/活动配置未变化。
- 外部工作站正常验证 TLS，12 种响应体与 VPS 工件逐字节相同；12 个 SVG 二维码均实际解码为对应订阅 URL。
- 外部 WSL sing-box 客户端分别通过 VLESS Reality、VMess WebSocket、Hysteria2、TUIC 和 AnyTLS 完成 HTTPS 请求，全部返回 204。
- 暂停定时器期间，订阅凭据和流量状态字节未变。

临时服务测试备份：`/root/sbctl-v016-acceptance/backup-20261020-184410/deployment.tgz`。归档摘要：`4be54432054b95aae1ab6baaba4fc89edbe7923bf63dd1176ad91d6835ceb0678`。包含私密配置，只保存在 VPS。

## 发布与最终安装

[Release Actions](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39476665755) 全部成功，包括 Debian 12、Ubuntu 22.04、Ubuntu 24.04 的 systemd 安装、非 root 服务、更新失败恢复和替换部署验收，真实 sing-box 1.10–1.14 与 Mihomo 新旧版本的检查，以及双架构生产信任根检查。[独立 CI](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39476658220) 也全部成功。

草稿重新下载的 amd64 二进制、SHA256SUMS 和签名 manifest 与 Actions 候选一致；VPS 原有 v0.0.14 与候选 v0.0.16 都成功验证该 manifest。其 sing-box 1.14.2 摘要与 VPS 已有内核相同。

临时部署验收后已完整恢复 v0.0.14 的文件、元数据和原定时器状态，并确认原 HTTPS 正常、临时恢复任务 inactive。随后发布 [sbctl v0.0.16](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.16)，8 个发布资产均来自上述同一次 Release 工作流。发布后再次下载 manifest，SHA-256 仍为 `7135c3c6845b8789ba28e6bd20bb558107de92ca7e89d3984c5cd6206406bebe`。

最终部署使用 `sbctl update --manifest ... --sbctl-artifact ... --sing-box-artifact ...` 的正式签名更新流程，不再手工替换程序。调用和恢复保护见 [upgrade_verified.py](upgrade_verified.py)。更新后运行中 `/proc/<sbctl MainPID>/exe` 与已测候选的 SHA-256 完全相同，版本为 0.0.16，实际 HTTPS 检查通过；配置、订阅凭据、暂停定时器时的流量状态和 sing-box 内核字节保持原样。

正式更新前另建备份：`/root/sbctl-v016-acceptance/backup-20261020-185337/deployment.tgz`；摘要 `e9be769696a8b41602c1a0c5a35730dca6a225d03d88336b5919e20524f31bb4`。备份只保存在 VPS，使用 15 分钟自动恢复保护。

正式更新后再次从外部工作站完成 12 种订阅体逐字节检查、流量头、12 个二维码解码、非法路径和正常 TLS 验证，并再次分别经五种协议完成 HTTPS 204 转发。通过后执行升级确认，恢复原定时器状态并停止自动恢复任务。最终运行 v0.0.16，保留 root-only 升级前备份。

## 边界

每个公网来源共享 8 个并发额度。ExternalProxy 必须由前置代理按真实来源限制连接。单来源隔离不能保证抵挡多个来源协同耗尽全局额度或耗尽网络带宽。
