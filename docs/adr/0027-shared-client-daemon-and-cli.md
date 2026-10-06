# 共享客户端后台与 sbcli

> **2026-10-04 注记（ADR-0028）**：本文"现有 UI 仍兼容 1.10–1.13"的承诺已被
> [ADR-0028](0028-client-pure-official-grpc.md) 取代——所有前端（sbcli/TUI/GUI）统一仅支持 1.14+ 内核与官方 gRPC 传输。

CLI、TUI、GUI 连接同一 sbclient 后台。后台独占数据目录锁、持有 client-core 引擎及内核进程；Unix domain socket／Windows named pipe 提供版本化同用户 IPC。前端退出仅断开，显式 stop 才停核并恢复工具拥有的系统代理设置。

新 sbcli 只支持 1.14+ 官方 gRPC，现有 UI 仍兼容 1.10–1.13。1.14+ 托管实例不再注入 Clash delay listener；只封装 URLTest 组测速。此决定替代 ADR-0026 的自定义单节点测速例外。

独立入口设置替代 UI 共享后台的互斥 TrafficMode：HTTP/SOCKS/Mixed、系统代理、TUN 分别配置，TUN 可与 Mixed 共存。旧 TrafficMode 保留为旧存储与界面的兼容投影；入口仍由引擎拥有，不由覆写文件拥有。此决定更新 ADR-0023 的入口模型，不改变覆写合并语义。

共享目录不自动采用旧 TUI/GUI 数据。显式迁移只接受空目标和停止的源目录，保留源数据，关闭自动启动和系统代理。远程 API 仅管理运行态，不冒充 ManagedService 或提供 SSH 生命周期操作。

异常回收沿用 Windows Job Object 和 Linux PR_SET_PDEATHSIG；macOS 没有现有等价保护实现，快照必须报告保护不可用。正常退出与信号会协作停止，强制终止后台的 macOS 回收仍待实现和实机验收。三平台完整支持以 .scratch/client-cli/acceptance.md 的门槛为准。
