# v0.0.17 命令和失败回滚边界验收

日期：2026-10-20。生产代码及 tag 提交：`d25f5621d584725b7b9deb058b0f97bf78eeca48`。

## 构建来源

[Release Actions 39481685855](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39481685855) 的生产 amd64 工件，注入生产公钥，未启用 test-signing。VPS 测试未使用本地编译的生产程序。

Actions 候选与 VPS 测试文件 SHA-256：`3733115c7f138c9e5f9d1b269e93e6ce5b8647053b7986db6304938fd015cc54`。

## 故障复现和修复验证

在 VPS 上用独立网络命名空间和临时目录运行 [verify_isolated.py](verify_isolated.py)。PATH 中代表宿主机的命令全是只写入临时标记的替身，不执行实际 systemctl、sysctl 或 Certbot；真实部署未参与故障注入。

旧版为 VPS 已安装的 Actions v0.0.16，摘要为 `b431e914ed814a467d9d864527717d185d7eae89e8f0df4e55b93798491e94bc`。

| 场景 | v0.0.16 | Actions v0.0.17 |
| --- | --- | --- |
| 隔离 root 缺少 systemctl 的 restart | 触及代表宿主机的 PATH 标记 | 失败，未触及 PATH 标记 |
| 隔离 root 缺少 sysctl 的 BBR | 触及 PATH 标记 | 失败，未触及 PATH 标记 |
| 隔离 root 缺少 Certbot 的 renew | 触及 PATH 标记 | 失败，未触及 PATH 标记 |
| 隔离 root 提供上述替身 | 优先执行 root 内替身 | 仍优先执行 root 内替身 |
| 真内核 check 后注入工件写入错误 | 原活动配置被删除 | 活动配置内容与 inode 均未改变 |
| 工件写入错误后 | 原工件字节不变 | 原工件字节不变 |

文件大小限制仅施加到隔离 CLI 子进程，并忽略该子进程的 SIGXFSZ，以使真实写入错误进入 Rust 错误处理；没有修改 VPS 磁盘限制。被校验的服务端配置通过 VPS 真实 sing-box 1.14.2 检查。

Rust 回归另外覆盖 Runtime 的 systemctl、sysctl、Certbot、UFW、账户命令缺失边界，单一程序名校验，以及 usr/bin 和 usr/sbin 的正常替身执行。

## 本地门禁和观测诊断

- 格式检查、所有目标/特性 Clippy、491 项常规 Rust 测试通过。
- 普通构建 release_trust 1 项及安装脚本 Python 单测 5 项通过。
- 32 测试线程的完整库测试连续五轮通过。此前偶发观测失败未稳定复现，仅增强错误断言和诊断，仍保留 [未解决记录](../review-connection-isolation/issues/03-observation-fixture-flake.md)。没有增加自动重试掩盖未知失败。

## 真实服务回归

复用已提交的 v0.0.15 live 验收脚本，将 v015 路径与 unit 前缀替换为 v017，在 VPS 暂存为 `/root/sbctl-v017-live.py`。开始前建立 root-only 备份并启动 15 分钟自动恢复任务。

候选实际启动后，12 种订阅和二维码响应、API 启停、服务账号的证书访问权限、服务端覆写编辑和 clear 后实际 PID 改变、非法覆写恢复均通过。测试未轮换真实订阅凭据，暂停定时器期间流量状态字节保持原样。

临时服务测试备份：`/root/sbctl-v017-acceptance/backup-20261020-191852/deployment.tgz`。摘要：`f99bbfda470037de6f7270420d51f69f19b56eadd046f22d1a5a2fe214c2d327b`。私密备份仅保留在 VPS。

外部工作站使用正常 TLS 校验，12 种响应体与 VPS 工件逐字节一致，流量头存在，12 个 SVG 二维码实际解码为正确订阅 URL，非法凭据与 query 返回 404。WSL 客户端分别经 VLESS Reality、VMess WebSocket、Hysteria2、TUIC 和 AnyTLS 完成 HTTPS 请求，均返回 204。

临时验收后完整恢复原 v0.0.16 的字节、元数据和定时器状态，确认真实服务及订阅正常，并停止临时恢复任务。

## 发布门禁和签名

[Release Actions](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39481685855) 和[独立 CI](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39481675978) 均成功。包括 Debian 12、Ubuntu 22.04、Ubuntu 24.04 的真实 systemd 安装、非 root 服务、失败更新恢复与替换部署，真实 sing-box/Mihomo 配置检查，以及双架构生产验签边界。

草稿重新下载的 amd64 程序摘要与实测候选一致。VPS 原有 v0.0.16 与候选 v0.0.17 均通过签名 manifest 验证。manifest-amd64.json 摘要：`b3d1303ea791dc122e1e6d79ae66d6805c9f7592760e55fd8f4465627b8d8997`；运行时 sing-box 1.14.2 的 `fc9c6e6ab345f045b16a0ed10d1ff28d68e8e56e7749fca30738d1406e98d7b8` 与现有内核一致。

发布页：[sbctl v0.0.17](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.17)。8 个资产均来自上述同一次 Release 工作流，没有重新编译或改写生产字节。

## 最终签名更新

发布后再次下载 manifest，其摘要仍与草稿一致。VPS 用正式 `sbctl update --manifest ... --sbctl-artifact ... --sing-box-artifact ...` 流程安装已验证文件。使用已提交的 v0.0.16 `upgrade_verified.py`，将 v016/0.0.16 替换为 v017/0.0.17，暂存为 `/root/sbctl-v017-upgrade.py`。

运行中的 `/proc/<sbctl MainPID>/exe` 与已测候选的摘要相同，版本 0.0.17；配置、订阅凭据、暂停定时器时的流量状态和 sing-box 内核字节保持原样。

升级前备份：`/root/sbctl-v017-acceptance/backup-20261020-192958/deployment.tgz`，摘要 `5d42f76e0df6d3a9f41f85c2d4cab0b894e25bf8695cc4467bcd5e9df3e6fdc97`，只保存在 VPS。升级期间另有 15 分钟自动恢复任务保护。

更新后再次完成外部 12 种订阅体、流量头、12 个二维码解码及非法路径检查；五协议转发再次全部返回 HTTPS 204。确认后恢复原定时器状态并停止自动恢复任务。最终版本为 0.0.17，三个实际服务均 active，sbctl 以专用非 root 账号运行，升级备份保留。

## 后续研究

另在两个独立的 WSL 临时目录中发现初始化工件写入会跟随父目录链接。此研究不属于当前生产修改，真实 UID 和竞态可达性尚待验证，记录于 [issue 03](issues/03-atomic-write-parent-symlinks.md)，不声称完成全部安全审查。
