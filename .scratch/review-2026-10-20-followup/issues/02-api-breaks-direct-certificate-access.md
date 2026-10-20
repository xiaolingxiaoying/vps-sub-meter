# P1：观测接口启停破坏 Direct HTTPS 证书权限

Status: needs-triage

位置：`src/cli/commands/update.rs:340`；后续调用 `src/lifecycle.rs` 的 `grant_self_signed_certificate_access`。

`apply_and_restart` 无论订阅模式为何都调用 `prepare_daemon_storage(root, false)`。Direct HTTPS 因而进入非 Direct 分支，将 pinned 证书和私钥递归改为 `root:sing-box 0640`。订阅服务账号属于 `sbctl-cert`，不属于 `sing-box`，服务重启后无法读取私钥，HTTPS 订阅不可用。配置快照恢复不恢复这些元数据，所以即使 API 操作失败也会留下故障。

## 隔离复现

`../reproduce_tls_permissions.py` 在一次性 chroot 内定义真实 Linux 账号/组，先确认 `sbctl` UID 65000、附加组 `sbctl-cert` GID 65002 可以读取 `0640 root:sbctl-cert` 的 pinned 私钥占位文件。运行 `sing-box api enable`，让 systemctl 桩失败并触发配置恢复。私钥 GID 变成 65001（sing-box），同一 UID/组组合随后无法读取。

脚本不使用宿主账号或真实服务，也未测试真实 TLS 握手；读取权限丢失由 Linux 实际权限检查确认。

## 修复方向

按当前订阅模式传入 direct 标志；失败恢复也覆盖存储权限。现有非根 `--root` 测试会跳过权限准备，不能覆盖这一缺陷。
