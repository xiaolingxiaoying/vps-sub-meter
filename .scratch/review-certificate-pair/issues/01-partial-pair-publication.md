# 第二个文件写入失败会留下混合代证书
Status: resolved
Type: research

certificate::pin 分别提交 fullchain.pem 和 privkey.pem。单文件原子性不能保证二者同一代。待使用只影响隔离 CLI 的 RLIMIT_FSIZE 在首个证书提交后使第二个密钥写入失败，检查是否保留旧可用对；复现后再设计成对发布和恢复，避免仅将错误吞掉或靠重试掩盖。

## Comments

2026-10-20（工作站时区）已用 v0.0.19 本地调试程序在临时 root 复现。首次成功 pin 后生成第二代有效证书对，再只限制隔离 CLI 的文件大小，使证书可写、较大的 RSA 私钥写入失败。返回 File too large，但 fullchain 已变、privkey 仍为旧字节，混合代被留在磁盘。复现脚本 [reproduce_partial_pin.py](../reproduce_partial_pin.py) 不影响宿主限制、服务或真实证书。现有 TLS 热重载可保留内存中的旧证书，但重启后无法加载混合对；后续需要独立设计成对提交与失败恢复，不在本轮文件路径边界修复中声称解决。

## Answer

v0.0.21 完整暂存两份材料、权限和同步后，通过 Linux 同父目录交换发布一代，提交前失败保留旧目录。VPS 的 Actions 产物经实际文件大小故障验证，旧 pair 完整保留，失败后服务重启能完成验证 TLS 和精确订阅响应，解除故障后重试成功；另一对文件和常规辅助文件保留。详见 [验收记录](../verification.md)。
