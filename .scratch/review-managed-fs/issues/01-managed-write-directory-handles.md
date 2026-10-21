# 托管原子写入和锁文件拒绝父目录链接
Status: resolved

从上一轮已复现的初始化越界写入继续。需要将临时创建、提交和清理固定到同一父目录句柄，避免路径预检查后的竞态；锁文件创建也不能落到指定目录之外。

## Answer

Unix managed_fs 使用 openat/mkdirat 固定无链接父目录，暂存及 renameat/drop 清理基于同一目录句柄；锁保留已有文件不带 O_CREAT 的 protected_regular 兼容行为。证书复用拒绝叶子链接，写入复用公共实现。501 项 Rust 测试、真实 UID 链接实验、首次 pin 双服务权限、三发行版 systemd 和 VPS Actions 生产产物检查通过。参见 [验收记录](../verification.md)。
