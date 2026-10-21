# 新建证书存储父目录缺少共享组遍历权限
Status: claimed

v0.0.18 候选的 Release Actions 39489569268 和独立 CI 39489539615 在真实 Debian 12 systemd Direct HTTPS 首次安装中失败。新建目录逐级 0700，而证书 pin 仅给主机子目录设置 0750；certificates 存储父目录仍为 0700，递归 chown 不会恢复遍历权限。旧的 UID 测试手工创建共享目录，未覆盖首次 pin。

需明确为证书存储根目录及主机目录应用 root:sbctl-cert 0750，继续使用无链接句柄；真实 UID 检查必须经 certificate verify 创建首次 pin，验证两个服务账号实际可读。
