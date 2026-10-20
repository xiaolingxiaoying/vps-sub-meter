# v0.0.15 修复及 VPS 验收

日期：2026-10-20。代码提交：`cb53a185a66387ad63dc7ef3edc44bcd6f5dfbdd`。

## 构建来源

[Release Actions run 39465390004](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39465390004) 的生产构建，注入仓库生产验签公钥且不启用 test-signing。本地未构建用于 VPS 验收的 release 程序。

| 程序 | Actions 产物、VPS 测试文件和运行中程序的 SHA-256 |
| --- | --- |
| sbctl 0.0.15 amd64 | `48ced52eb14628ead81ceaa6424cc67b440935802b557f9f5f3b7154e449ee7e` |
| sing-box 1.14.2 amd64 | `fc9c6e6ab345f045b16a0ed10d1ff28d68e8e56e7749fca30738d1406e98d7b8` |

VPS 原有 sing-box 与本次 Actions 下载、验证并随包发布的运行时逐字节一致，因此测试期间未替换内核程序。

## 验证结果

| 验证 | 结果 |
| --- | --- |
| 本地 Rust 门禁 | fmt、所有目标/特性的 Clippy、481 个常规测试通过。 |
| 普通构建验签边界 | release_trust 1 个测试通过；安装脚本 Python 单测 5 个通过。 |
| 真内核配置兼容性 | sing-box 1.10–1.14 和 Mihomo 当前/1.18 的 7 个测试通过；最终提交的 Actions 对应两个 job 也通过。 |
| VPS 隔离回归 | Actions 候选程序验证 API 失败恢复与并发轮换串行、append/replace、服务端覆写清除激活、编辑草稿隔离，全部通过；配置检查使用 VPS 真实 sing-box。 |
| VPS 真实权限 | 一次性 chroot 使用实际 Linux UID/GID 验证 API 恢复保留 `root:sbctl-cert 0640`，订阅服务账号仍可读取私钥。 |
| memory 流响应 | 独立真实 sing-box 的连续 chunked memory 流能正常返回首个快照；真实部署启停 API 均完成且 HTTPS 可用。 |
| 真实服务覆写 | 编辑临时服务端日志层后配置生效且 sing-box PID 改变；clear server 后层被删除、默认日志恢复，PID 再次改变，HTTPS 保持可用。 |
| 非法覆写恢复 | 不存在的 outbound 被真内核拒绝，未留下源文件，活动配置与部署配置字节未变。 |
| 公网订阅 | 外部工作站使用正常 TLS 校验；12 个订阅响应体与 VPS 工件逐字节一致，流量头存在，总览 200，非法凭据和 query 返回 404。 |
| 公网二维码 | 12 个 SVG 均实际解码，内容与对应私有订阅 URL 一致。 |
| 五协议实际转发 | 外部 WSL sing-box 1.14.1 客户端分别经 VLESS Reality、VMess WebSocket、Hysteria2、TUIC、AnyTLS 完成 HTTPS 请求，均返回 204。私密节点配置仅驻留内存和自动清理的私有临时目录。 |
| 数据保留及恢复 | 真实订阅凭据未轮换，流量状态在暂停期间保持原字节；结束时恢复 sbctl 0.0.14、全部原文件和元数据，恢复原定时器状态，公网订阅复核通过，回滚定时器 inactive。 |

## 备份与首次验收发现

最终测试备份保留在 VPS 的 root-only 目录：

`/root/sbctl-v015-acceptance/backup-20261020-172720/deployment.tgz`

归档 SHA-256：`e9c1e84dc45af86879a395bd2b05b31b28c4c7bfb28d40043a9805601b33c2c05`。备份包含私密配置，未下载或提交。

首次候选 `60c61d1` 在真实 API 启用时发现 `/memory` 连续流读取不结束。测试外层在 100 秒后终止命令，随后自动恢复并核验部署。该候选未发布；新增首个 JSON 快照读取和总期限修复后，重新运行 Actions，并完成上表所有 VPS 测试。首轮备份也保留在同一 root-only 父目录下。

## 发布门禁

[最终提交的 CI](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39465377681) 与上述 Release 工作流均成功，包括 Debian 12、Ubuntu 22.04、Ubuntu 24.04 的真实 systemd 验收、非 root 服务、安装/更新失败恢复、两个架构的生产构建与验签信任根门禁。

从最终草稿重新下载 sbctl amd64、签名 manifest 和 SHA256SUMS，再次确认二进制与 VPS 测试的 `48ced52e…449ee7e` 完全相同，manifest 的摘要及随包运行时摘要也相同。VPS 上候选程序和恢复后的原 sbctl 0.0.14 均成功验证新 manifest 的生产签名，证明现有安装的信任根可以接受这次发布。

发布页：[sbctl v0.0.15](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.15)。全部八个资产均来自同一次成功的 Release 工作流；验收未重新构建或改写生产二进制。
