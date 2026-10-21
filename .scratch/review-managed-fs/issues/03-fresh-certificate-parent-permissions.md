# 新建证书存储父目录缺少共享组遍历权限
Status: resolved

v0.0.18 候选的 Release Actions 39489569268 和独立 CI 39489539615 在真实 Debian 12 systemd Direct HTTPS 首次安装中失败。新建目录逐级 0700，而证书 pin 仅给主机子目录设置 0750；certificates 存储父目录仍为 0700，递归 chown 不会恢复遍历权限。旧的 UID 测试手工创建共享目录，未覆盖首次 pin。

需明确为证书存储根目录及主机目录应用 root:sbctl-cert 0750，继续使用无链接句柄；真实 UID 检查必须经 certificate verify 创建首次 pin，验证两个服务账号实际可读。

## Answer

e62d15a 显式对存储根目录和主机目录应用 root:sbctl-cert 0750。新增真实首次 pin 的检查在 v0.0.18 生产候选上失败，在 v0.0.19 Actions 产物上通过；两个服务 UID 读访问及 API 恢复均通过。独立 CI 与 Release 的 Debian 12/Ubuntu 22.04/Ubuntu 24.04 systemd 门禁均成功。v0.0.18 未发布，未改写 tag。参见 [验收记录](../verification.md)。
