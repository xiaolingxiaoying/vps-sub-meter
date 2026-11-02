# 第二轮 Linux、VPS 与真实数据平面验收

Type: task
Status: resolved

运行格式、严格 Clippy、全量 Rust/脚本门禁和固定真实核心矩阵。VPS 用最终无 test-signing 候选在私有网络命名空间执行订阅/菜单/管理边界验收；如离线五协议原型成功，一并执行真实 SOCKS 转发测试。测试前后核对现网文件、权限、服务 PID/状态及路由，清理测试进程和临时文件。本机代理、DNS 等保持不变。

## 结果

- WSL2 Ubuntu 22.04：fmt、严格 workspace/all-targets/all-features Clippy、565 项常规 Rust 测试、7 项真实核心检查、无 test-signing 发布信任测试、5 项 Python 单测和 Node 复制交互回归通过。菜单、安装向导、扩展 GET/HEAD smoke 全部通过。
- CLI 测试 helper 两处编译期 cargo_bin 宏改为运行时解析，以支持在另一台 Linux 主机用相同测试程序检验明确指定的候选。该修正后再次运行全部 182 项 CLI 测试通过。首次 VPS 尝试因编译路径不可用失败，临时目录已清理、现网核对不变；以下为修复后的完整重跑结果。
- 最终无 test-signing Linux release SHA-256：`5f4d75cb333f68a40d8fe2c50277b9224221a2faea1064fdef69a98b9142725a`，本地与 VPS 上传后相同。
- VPS Ubuntu 22.04 / sing-box 1.14.2：13 订阅+13 QR+index 的 GET/HEAD/错误边界 smoke、21 项生产候选管理回归（14 凭据+4 状态+3 重启）、菜单及安装向导均通过。测试程序在临时私有目录，通过运行时指定的普通 release 候选执行，不替换现网程序。
- 五协议离线数据平面在 WSL 真实 1.14.1 与 VPS 真实 1.14.2 均通过。每个原始客户端 outbound 通过独立 SOCKS5 客户端转发 HTTP 并验证固定响应正文；仅使用 127.0.0.1 和独立 network namespace。服务端仅收紧监听地址和重定位 Reality decoy 到本地 TLS 1.3 服务。脚本拒绝初始 network namespace，正反向检查通过；已接入 CI sing-box-profiles job。
- VPS 现网二进制、配置、工件、权限、服务 PID/状态/重启计数、路由及路由规则前后相同；测试程序、脚本、监听器和临时凭据均已清理。开发主机代理、WinHTTP 和 DNS 对比相同，无主机路由、网卡或防火墙修改。

离线转发证明实际协议互通，不证明公网可达、真实域名 Reality 伪装、公有 CA 信任、生产 ACME 或三发行版真实 systemd 发布门禁。本轮未发布或永久升级 VPS。
