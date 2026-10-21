# v0.0.19 托管文件边界验收

日期：2026-10-20（工作站时区）。路径边界提交 `0be60d4aa6076ad5f856659b0443094d96e7a281`；首次证书目录权限修复及 v0.0.19 tag 提交 `e62d15ac0cde214fd160f015010d4bb51f883a24`。

## 已复现问题

VPS 原有 Actions v0.0.17 摘要 `3733115c7f138c9e5f9d1b269e93e6ce5b8647053b7986db6304938fd015cc54`。使用 [真实 UID 检查](../../scripts/dev/check-managed-fs-boundaries.py) 在两个临时树中，由 UID 65000 放置 artifacts 父目录链接或自签名 key.pem 链接。该 UID 无权读写 root-only 哨兵。旧版 root 执行 config init 时两个哨兵均被修改；没有对真实部署或系统文件制造链接。

artifacts 场景模拟服务账号拥有的工件目录；私钥场景刻意提供攻击账号可写的 SNI 目录，验证辅助写入函数在此条件下的防护，不声称默认 root 拥有的证书目录可被普通账号写入，也不把这些隔离实验当作远程代码执行证据。

Unix 新实现逐级无链接打开目录，临时文件创建、提交、失败清理及删除相对于固定句柄。确定性 Rust 回归在暂存后将原父目录改名、原位置替换成外部链接，验证提交与失败清理只作用于原目录；权限策略失败保留旧字节，已存在 1771 目录保持原权限。另覆盖叶子链接替换与删除、过期工件清理、锁、非法文件名和证书链接。

## v0.0.18 门禁失败与修复

[Release 39489569268](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39489569268) 和[独立 CI 39489539615](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39489539615) 在 Debian 12 真实 systemd 首次 Direct 安装失败：certificates 父目录逐级创建后仍为 0700，只有主机目录被设置为共享可遍历。服务 active，但 TLS 无法读取证书。工作流因此跳过 package，v0.0.18 未发布，VPS 保留 v0.0.17。

此失败候选的生产 amd64 摘要为 `62291a64efacc15115dc65a0a4ae1afabb6e5f735c6da284a2638d73aedbe801`。在 VPS 临时树中，其链接拒绝检查通过，而新增首次 pin 的 chroot/UID 检查明确失败（共享父目录缺少 gid/遍历权限），与 Actions 根因一致。

修复显式为证书存储根目录和主机目录设置 root:sbctl-cert 0750，仍经无链接文件句柄。[证书权限脚本](../../scripts/dev/check-api-certificate-permissions.py) 现在使用 openssl 生成临时有效证书、经真实 certificate verify 首次 pin，确认 sbctl 与 sing-box UID 均可读，再检查 API 回滚保留 root:sbctl-cert 0640。没有手工修好 pinned 目录再假设首次安装正常，也没有修改或重用旧 tag。

## 本地验证

- v0.0.19 格式、全目标全特性 Clippy、501 项常规 Rust 测试通过；Windows cargo check --locked --lib 通过。
- 首次 pin 的两个服务 UID 访问、API 失败恢复、真实 UID 链接拒绝、root 哨兵完整性及正常初始化通过。
- 普通构建 release_trust 1 项与安装脚本 Python 单测 5 项在文件边界修改后通过；完整发布工作流还会对最终提交执行。

## 发布与 VPS 验收

[Release 39540451965](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39540451965) 和[独立 CI 39540448011](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39540448011) 均成功。包含 Debian 12、Ubuntu 22.04、Ubuntu 24.04 的真实 systemd 安装、非 root 服务启动、失败更新恢复与替换部署、真实 sing-box/Mihomo 配置检查及双架构生产信任锚验证。

生产 amd64 候选已取自同一次 Release Actions（生产公钥、无 test-signing），摘要 `fd114c3c1c5902873fec707672e19db256bf019039a9bb44e539edbd548837df`；VPS 摘要一致。该产物通过真实 UID 链接拒绝、root 哨兵字节/权限保护、正常初始化、首次证书 pin 双服务 UID 可读和 API 回滚权限检查。

真实服务测试使用已提交的 v0.0.15 live 脚本，仅将 v015 路径/unit 前缀改为 v019，暂存 `/root/sbctl-v019-live.py`。开始前暂停原定时器、建立 root-only 备份，并启动 15 分钟定时恢复保护。候选实际启动后，12 种订阅及 QR 响应、API 启停的证书读取权限、服务端覆写 edit/clear 的真实进程重启、非法覆写保留旧配置、订阅凭据和暂停定时器期间的流量状态均通过。

临时验收备份 `/root/sbctl-v019-acceptance/backup-20261021-035027/deployment.tgz`，摘要 `026433683e466c22a1250f37864384f4354f0011a86c796fa654e29ff5103088`，只保留在 VPS。外部工作站以正常 TLS 验证 12 种响应体与 VPS 工件逐字节一致、流量头、12 个二维码实际解码、overview 及非法凭据/query。独立 WSL 客户端经 VLESS Reality、VMess WebSocket、Hysteria2、TUIC、AnyTLS 分别完成 HTTPS 请求，全部 204。

临时验收后已完整恢复原 v0.0.17 的文件字节、元数据和定时器，真实服务及公开订阅检查通过，临时恢复 guard 已停止。

草稿重新下载的 sbctl-linux-amd64 与实测候选摘要一致，签名 manifest 内程序摘要一致。manifest-amd64.json 摘要 `0da78fcdc90d7e9a73e38f047cc06f204a42686511cec32a865bff5747a343ac`；固定 sing-box 1.14.2 摘要 `fc9c6e6ab345f045b16a0ed10d1ff28d68e8e56e7749fca30738d1406e98d7b8`，与 VPS 现有内核相同。

原 v0.0.17 与候选 v0.0.19 均通过此 manifest 的生产签名检查。[v0.0.19 发布页](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.19) 已公开，8 个资产来自同次 Release 工作流；公开后重新下载的 manifest 摘要仍一致，没有重编译或改写实测生产字节。

## 正式签名升级

使用已提交 v0.0.16 upgrade_verified.py，仅将 v016/0.0.16 改为 v019/0.0.19，暂存 `/root/sbctl-v019-upgrade.py`。重新备份并启动 15 分钟恢复任务后，经正式 sbctl update --manifest/--sbctl-artifact/--sing-box-artifact 流程安装。运行中 `/proc/<sbctl MainPID>/exe` 摘要与实测候选一致，版本 0.0.19；配置、订阅凭据、暂停定时器期间流量状态和 sing-box 内核字节保持原样，12 种本地订阅及二维码响应通过。

升级前备份 `/root/sbctl-v019-acceptance/backup-20261021-040043/deployment.tgz`，摘要 `2a6c843bb8aa4e519ead951ad456929ddb53e65cfc564a914b63a7856dc9ccdc`，只保留在 VPS。

正式升级后再次完成工作站外部 TLS、12 种精确响应体、流量头、12 个二维码实际解码、非法路径及五协议 HTTPS 204，全部通过。

确认后运行 commit，原定时器状态已恢复，升级恢复 guard 为 inactive，私密备份保留。最终版本 0.0.19，sbctl.service、sing-box.service 和 sbctl-http.socket 均 active，sbctl 服务账号为非 root 的 sbctl。

## 范围与后续

生产竞态保证针对 Unix 的固定目录句柄；Windows 开发适配只增加父目录 reparse/link 检查，不声称具备同等竞态防护。已存在的实际目录不由通用创建器重新赋权；证书共享目录单独应用明确策略。托管父目录符号链接现在被拒绝，使用这类链接的部署需换成实际目录。

观测夹具曾有一次未稳定复现的失败仍保持 [needs-triage](../review-connection-isolation/issues/03-observation-fixture-flake.md)。另外独立复现了[成对证书发布失败](../review-certificate-pair/issues/01-partial-pair-publication.md)，后续继续修复；不据此声称所有审查问题已解决。
