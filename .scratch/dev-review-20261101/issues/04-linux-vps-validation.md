# Linux 与授权 VPS 隔离验证

Type: task
Status: resolved

## 实现

- 修复状态 JSON 测试以当前夏令时偏移推算月初的错误预期，按月初当地时间计算；产品账期实现未改变。
- 补充 verify-review.py：临时私有目录、只读/拒绝写操作的命令替身、回环订阅监听、格式与 QR 矩阵、路由边界、只读状态和九个 CLI 错误入口脱敏。
- CI 增加无依赖 Node 复制交互回归和隔离 smoke。
- 修复 Unix 专用 Path 测试导入造成的 Windows 未使用警告。

## 验收

WSL fmt/Clippy/545 项常规测试、7 项固定真实内核检查、无 test-signing 信任测试通过。最终显示增量后重跑 fmt/Clippy、19 项 binary 单测、索引 CLI 定向测试和 release 菜单/smoke 全过。另有 5 项 Python 单测及无依赖 Node 复制交互回归通过。Docker 引擎未启动，使用 WSL2。

最终候选 SHA-256：`4f6fadd1e87dec5fecb822999df0651f12c9e68bd3330d3dede9dcdb3f22aa92`。本地与 VPS 上传摘要一致。授权 VPS 为 Ubuntu 22.04 / systemd，使用现有真实 sing-box 1.14.2，仅执行 version/check。候选在私有 network namespace 及临时 0700 目录验证：13 格式和 13 QR、索引、非法认证/路由/query/方法、状态与工件只读、九个 CLI 错误入口脱敏、完整菜单与安装向导 PTY，全部通过。

前后比较确认现网二进制、配置、工件、权限、两服务 PID/状态/重启计数、路由及规则均不变。VPS 候选与临时脚本/fixture 已清理。本机系统代理、WinHTTP 代理、DNS 前后相同；没有执行主机路由、网卡或防火墙修改。

本轮未运行 Docker 三发行版真实 systemd 生命周期、真实生产 ACME、五协议端到端转发或签名发布，不将隔离 smoke 描述为这些发布级证明。
