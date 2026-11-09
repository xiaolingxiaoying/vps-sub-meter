# VPS 协议验收：2026-11-09

本次使用管理员明确授权覆盖的 Ubuntu 22.04.5、x86_64、真实 systemd 测试 VPS。服务器地址、SSH 密码、订阅凭据和业务配置内容不写入本报告或仓库。

## 构建与传输

严格采用 Windows 源码 → WSL2 Ubuntu 22.04 原生文件系统 → Cargo release 编译 → 原生程序返回 Windows → 上传 VPS 的流程。VPS 未运行 Cargo/rustc；远端 Python 仅作为已有系统中的验收驱动，不是新增生产服务依赖。

新增 `scripts/dev/build-vps-candidates.sh` 在 WSL 同步源码、构建管理程序和两个测试可执行文件，并复制至 Windows 工作区 `.scratch/acceptance-bin/`。上传后逐一校验 SHA-256，只展开预期文件名。运行的 Xray/Mihomo 为官方预编译发布，未在 VPS 编译。

最终部署的 sbctl SHA-256：`89a7ad403871794189855eeac95d281157f813f1a2f3378e62b12aa34cec2df7`。

完整脱敏结果见 `.scratch/runtime-templates-xray/vps-acceptance-20261109.json`。私有原始日志与程序/配置备份保留在 VPS 的 `/root/sbctl-protocol-test-18c79cb-20261109`、`/root/sbctl-protocol-test-18c79cb-round3` 和 `/root/sbctl-protocol-test-18c79cb-rollback` 下，目录仅 root 可读。

## 互通结果

所有以下测试均在真实 systemd 的 DynamicUser / NoNewPrivileges 环境运行。TLS 使用一次性证书的指纹或明确信任该证书，未依赖关闭验证。

| 服务端 | 客户端 | 组合 | 结果 |
| --- | --- | --- | --- |
| Xray 26.10.31 | Xray 26.10.31 | VLESS XHTTP TLS | HTTP 转发通过 |
| Xray 26.10.31 | Xray 26.10.31 | VLESS WS TLS | HTTP 转发通过 |
| Xray 26.10.31 | Xray 26.10.31 | Trojan TCP TLS | HTTP 转发通过 |
| Xray 26.10.31 | Xray 26.10.31 | VLESS XHTTP Reality | 外部目标握手及 HTTP 转发连续三轮通过，约 7.71 / 6.12 / 5.80 秒 |
| Xray 26.10.31 | sing-box 1.14.2 | VLESS WS TLS / Trojan TCP TLS | 两种组合 HTTP 转发通过 |
| Xray 26.10.31 | Mihomo 1.19.33 | VLESS XHTTP TLS / VLESS WS TLS / Trojan TCP TLS | 三种组合 HTTP 转发通过 |

Xray 配置真核用例另覆盖前期 6 份配置及四组合的 8 份服务端/客户端配置。测试的代理入口与 HTTP 源站均使用 VPS 回环地址临时高端口；Reality 额外访问显式选择的公网 TLS 目标。不能将此结果称为“所有公网入站/防火墙已验收”，也不能等同于 GUI 客户端导入测试。

## 覆盖部署与故障恢复

先备份原管理程序、配置、服务定义和订阅产物，再原子替换管理程序并重启 sbctl。现有 sing-box 数据平面保持运行，配置文件与内核二进制摘要不变。覆盖前后检查 12 个订阅格式加概览共 13 个入口，均返回 200；带查询参数的无效订阅路径继续返回 404。

再注入一个只在 `serve` 启动时退出 23 的候选程序。验收驱动检测失败，恢复上一程序、清理失败状态并重启服务；恢复后的程序摘要与失败注入前一致，13 个入口再次全部通过。最终 sbctl.service 与 sing-box.service 均 active。

这验证的是管理程序覆盖和启动失败恢复，**不是尚未实现的 sing-box/Xray 正式内核切换事务**。

## 实际发现与修正

- 外部 Reality 握手不能复用纯回环测试的 10 秒预算。增加显式外部目标用例和独立 30 秒预算，避免将握手/stream-one 等待误判为协议不可用。
- Mihomo 1.19.33 支持 XHTTP TLS，但官方不承诺当前 Xray 26.10.31 的 Reality 互通。生成器与 `mihomo-xhttp` 导出增加拒绝/过滤，并补兼容边界回归测试。
- 保留默认私网目标保护；只有隔离测试配置为回环 HTTP 源站提供例外。
- 根配置和真实凭据不进入验收输出，远端详细日志与快照保持私有。

## 可复现入口与限制

在 Windows 启动 WSL 构建：

```powershell
wsl -d Ubuntu-22.04 -- bash /mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/scripts/dev/build-vps-candidates.sh
```

将返回 Windows 的程序、测试可执行文件、已验证的 Xray/Mihomo 工件上传至 `/opt/sbctl-protocol-test-<标识>/`，在 VPS 上运行验收驱动：

```bash
python3 vps-presets-accept.py \
  --artifacts /opt/sbctl-protocol-test-example \
  --evidence /root/sbctl-protocol-test-example \
  --client-matrix
```

可额外指定 `--reality-target host:port` 测试外部握手。只有显式传 `--activate` 才覆盖管理程序；`--activate --inject-startup-failure` 执行失败注入与恢复。每次激活须使用新的 evidence 目录，避免覆盖恢复点。

本轮本地 fmt、Clippy、workspace/test-signing 回归及最终协议/CLI 定向测试通过。候选构建未注入生产发布公钥，不作为正式签名 Release 分发；签名升级、跨架构、完整内核切换、GUI App 导入和公网 CDN 路径仍需单独验收。原部署保持 sing-box，新增四组合仍通过候选命令生成，尚未接入正式订阅发布事务。
