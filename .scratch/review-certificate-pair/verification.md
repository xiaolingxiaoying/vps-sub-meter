# v0.0.21 证书成对发布验收

日期：2026-10-20（工作站时区）。目录代际提交 `35e23dbca8d214c5cee0260e733dd11cee2c4ace`；核心刷新及 v0.0.21 tag 提交 `e221dff9c999317c73664fc751fd9e3e596d55c4`。

## 故障及拒绝发布的候选

VPS 原 Actions v0.0.19 的摘要 `fd114c3c1c5902873fec707672e19db256bf019039a9bb44e539edbd548837df`。通过 [check-certificate-generation.py](../../scripts/dev/check-certificate-generation.py) --old，在临时部署树向隔离 CLI 注入 RLIMIT_FSIZE，仅使较大的 RSA 私钥写入失败。旧版返回 File too large，但证书已更换、私钥保留旧字节，留下混合代。真实部署没有参与故障注入。

v0.0.20 候选摘要 `ba64b7aa8ba244b5e0f534aa3d647391a2cc1981d234e868d064c0cd55734e88`。其 [Release Actions 39546788910](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39546788910) 虽成功，VPS 的补充真实内核实验仍发现目录交换使 sing-box 1.14.2 的 watcher 失效：磁盘已为新证书，原进程仍通过旧证书握手。使用 unshare --net、临时证书和独立 core 进程，没有修改真实服务。此候选保留为未发布草稿，未改写 tag；后续 v0.0.21 加入真实核心刷新门禁。

## 实现与本地验证

Linux 先完整写入、设置权限并同步暂存目录，再原子交换主机目录；失败保留旧代。固定目录读两份 PEM，目录设备/inode 纳入 TLS stamp。其他常规文件通过相对句柄硬链接保留内容、属主、权限和 inode（ctime 可变化）；链接、子目录和特殊文件拒绝发布。每份材料最大 1 MiB，目录最多 1024 项。

未变化的材料和权限不交换目录。新代包含待刷新标记；特权 CLI 只对具有正确部署和单元标记的 sing-box 执行 try-restart。正常顺序下，刷新失败或发布后退出留下的标记可由后续同证书重试处理。成功后确认；管理员已停止的服务不被启动。该操作会短暂中断既有代理连接，不声称零停机。证书发布锁保持 root 拥有、0600，服务账号不能读取或 flock 它。

本地格式、所有目标/特性 Clippy、512 项常规 Rust 测试及 Windows cargo check --locked --lib 通过。真实 UID 的首次 pin、API 回滚及目录/私钥链接拒绝检查通过。确定性回归覆盖第二个文件或目录权限失败、父目录被替换、提交目标变化、辅助文件、已打开读句柄、并发读不接受混合代、相同时间戳及未变化 pin 保留目录。

私有网络命名空间内，故障后新启动的 sbctl 完成带证书校验的 TLS 握手并返回精确订阅内容；解除写入故障后再次启动也成功。真实 sing-box 1.14.1 在新代后提供新证书，失败和未变化 pin 不重启。进一步强制刷新失败：CLI 报告已发布但刷新失败，待刷新标记保留；解除故障后，同证书重试不再交换目录但完成核心刷新。

## Actions 与 VPS 验收

[Release 39548710160](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39548710160) 与[独立 CI 39548704272](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39548704272) 均成功。包括三发行版 Debian 12 / Ubuntu 22.04 / Ubuntu 24.04 的真实 systemd 安装、服务身份、失败更新恢复与替换部署；发布锁在自签名授权与更新恢复后仍为 root 0600，两个服务账号均不可读。真实核心刷新实验已纳入 sing-box-profiles 门禁。

生产 amd64 候选来自同次 Release 工作流，注入生产公钥且无 test-signing，摘要 `abde499903b65afee70c0f3b5035cedf188d85d60152970d81fdcd7c71b42955`。后续验收只使用该 Actions 程序，不以本地调试构建替代生产测试。

VPS 文件摘要一致。此产物与安装的真实 sing-box 1.14.2 在私有网络命名空间中通过：第二个文件真实写入故障保留旧对、故障后和正常重试后的 TLS 启动与精确订阅、新代实际核心提供新证书、失败或未变化 pin 不重启、刷新失败后的持久化标记及同证书重试；另通过首次 pin 的真实服务 UID 访问、API 回滚权限，以及目录/私钥链接拒绝。命名空间实验没有触及真实服务和真实证书。

真实部署测试复用已提交 v0.0.15 live helper，将 v015 改为 v021，并在初次启动后调用 [verify_live_generation.py](verify_live_generation.py)。新 helper 验证同一有效证书的 pin 不改变目录或核心 PID、证书和常规辅助文件保持原字节/权限，再显式重启服务验证读取；两个真实服务账号均能读 key、不能读 root-only 发布锁。还在恢复路径中清理基线未存在的新发布锁，以保持旧版部署的原文件集合。

临时测试备份为 `/root/sbctl-v021-acceptance/backup-20261021-052444/deployment.tgz`，摘要 `c71fb6816699b1b535192910249ad21caf4250b3a589fdd0eb6d20823a4c5c74`，只保留在 VPS。15 分钟恢复任务保护整个临时测试。实际候选进程通过 12 种订阅及 QR、本机 TLS、API 启停证书权限、服务端覆写 edit/clear 的真实进程重启、非法覆写回滚，配置凭据及暂停定时器期间的流量状态保持原样。

工作站外部正常 TLS 校验通过，12 种订阅内容与 VPS 工件逐字节一致、流量头存在、12 个 SVG 二维码实际解码为相应 URL，overview 与非法凭据/query 检查通过。独立 WSL 客户端经 VLESS Reality、VMess WebSocket、Hysteria2、TUIC、AnyTLS 完成 HTTPS 请求，全部 204。

临时验收后恢复原 v0.0.19 的文件字节、元数据和定时器，清理原本不存在的新增发布锁，真实服务和公开订阅再次通过，自动恢复 guard 为 inactive。

草稿 manifest-amd64.json 摘要 `eee9e23cb16ab9eaa3de2cfd1a0ecfbc3445e4e677f8f496c5920ca4a2aaf012`。清单内程序摘要及 GitHub 对发布资产计算的 digest 均与 VPS 实测候选相同；固定 sing-box 1.14.2 摘要 `fc9c6e6ab345f045b16a0ed10d1ff28d68e8e56e7749fca30738d1406e98d7b8` 与现有核心相同。Windows 草稿程序的重复下载未收到首字节，已取消该单次连接；完整 JSON、资产 digest 和原 Actions 文件核对正常，公开后再由 VPS 下载程序核对。

原 v0.0.19 和候选 v0.0.21 均通过生产签名 manifest 验证。[v0.0.21 发布页](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.21) 的 8 个资产来自同次工作流。公开后 VPS 直接下载程序与清单，二者 SHA-256 再次与已验收候选及草稿清单一致，未改写或重编译生产字节。

## 正式签名升级

使用 v0.0.16 upgrade_verified.py 模板，将 v016/0.0.16 改为 v021/0.0.21，并在 begin 的签名更新后调用 verify_live_generation.py。root-only 备份与 15 分钟恢复任务保护升级，经真实 sbctl update --manifest/--sbctl-artifact/--sing-box-artifact 流程安装；程序参数使用上述重新公开下载的已验证文件。

签名升级及附加真实部署证书验证通过，运行中 `/proc/<sbctl MainPID>/exe` 摘要与已验收的 Actions 程序一致，版本 0.0.21。配置、订阅凭据、暂停定时器期间流量状态及 sing-box 核心字节均保持原样，本地 12 种订阅及 QR 响应再次通过。升级前备份 `/root/sbctl-v021-acceptance/backup-20261021-053853/deployment.tgz`，摘要 `0386761b46dd676ab066f2d3bcea7b141b11eaee3fce65971fe001f429c0f1a2`，仅保留在 VPS。

正式升级后再次通过工作站外部 TLS、12 种逐字节订阅内容、流量头、12 个二维码实际解码和非法路径检查；五种协议外部 HTTPS 再次全部返回 204。

确认后已 commit：恢复原定时器状态，自动恢复 guard 为 inactive，私密备份保留。最终版本为 0.0.21，sbctl.service、sing-box.service、sbctl-http.socket 均 active，两个进程分别以非 root 的 sbctl / sing-box 账号运行。当前工作不将整个长期审查目标标为完成；并发确认边界和旧观测夹具问题继续跟踪。

## 限制及后续

生产原子交换保证为 Linux 本地文件系统；非 Linux 开发适配不声称同等竞态/崩溃保证。旧代读句柄若尚未打开第二份 PEM 就被清理，可能暂时读取失败；不会接受混合代，原内存中的有效证书仍可服务。异常退出可能留下私有暂存目录。

两个特权发布命令并发时的刷新确认代际绑定仍需 [专门故障注入](issues/05-refresh-ack-concurrency.md)，不将当前顺序重试测试当作该并发问题已解决。此前观测夹具的 [偶发失败](../review-connection-isolation/issues/03-observation-fixture-flake.md) 也继续保留，未用自动重试掩盖。
