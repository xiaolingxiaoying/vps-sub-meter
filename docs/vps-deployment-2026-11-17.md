# 三台 VPS 部署与安全核查

日期：2026-11-17（Asia/Shanghai）。依据外部 `VPS-DIAGNOSIS-2026-11-17.md` 和本机私有连接说明执行，凭据不写入本报告。

三台均已部署当前工作树的 Linux amd64 候选，版本显示 `sbctl 0.0.25`；这是本地构建，未作为正式 Release 发布。保留用户原有未提交改动，没有重装或升级代理内核。

最终程序 SHA-256：

```text
50466d95254e17dcef98593c67418cb01555f7e9ec3f70aebb7d02c0b865c50c
```

构建注入仓库配置的生产**公开**信任公钥，成功验证官方 `sbctl-v0.0.25` 清单，并拒绝开发签名及错误 schema；未使用生产私钥，也没有给本地候选伪造发布签名。

## 最终结果

| 项目 | GTT | NTT | VMISS |
| --- | --- | --- | --- |
| 保留的部署 | legacy，5 节点 | legacy，5 节点 | dual，9 节点、原 2 组 |
| 原有 sing-box | 1.14.2 | 1.14.4 | 1.14.2；Xray 同样保留 |
| VPS 自身逐节点 HTTPS 转发 | 5/5，204 | 5/5，204 | 9/9，204 |
| 当前本机出口逐节点直连 | 5/5，204 | 5/5，204 | Hy2/TUIC 204；Clash 中 6 个 TCP 节点超时 |
| VPS 本地订阅 GET / HEAD | 各 13 项 | 各 13 项 | 各 29 项；另 10 项不兼容格式返回 404 |
| 二维码 | 1 项 | 1 项 | 3 项 |
| 当前本机出口订阅 HTTPS | 证书验证通过，200 | 证书验证通过，200 | TCP 443 超时，未进入 TLS |
| 最终项目服务 | 正常、非 root | 正常、非 root | 三个服务正常、非 root |
| 根分区使用率 | 44% | 41% | 88% → 81%，可用约 1.9 GiB |

VMISS 的 XHTTP Reality 不在当前 Clash 订阅中，因此本机 Clash TCP 测试只有 6 项；服务端逐节点测试包含它。VPS 自身测试不能证明当前本机到 VPS 的直连路径恢复。

所有最终订阅均校验 `no-store`、流量元数据、HEAD 长度、无效凭据与查询参数拒绝；完整 sing-box 配置由实际内核检查。当前本机出口使用独立 Mihomo 实例，关闭 TUN，绑定真实 WLAN，强制 VPS IPv4 并保留原 SNI；TCP 握手和订阅 HTTPS 也绑定同一出口，排除现有代理/TUN 中转。临时客户端均已退出。

## 已修复和保留

1. 在三台的 `/etc/sbctl/overrides/sing-box-server.d/20-local-dns.json` 持久保存活动 DNS/IPv4 定制，避免下次 regenerate 丢失。GTT 的本地 resolver、Reality handshake resolver 和既有全球 IPv6 拒绝行为均保留。
2. 在 `90-egress-guard.json` 持久保存私网、回环、链路本地及 Fake-IP 拒绝规则。使用 `rules_mode: append`，不复制自动生成的 resolve 规则。三台均用真实节点验证：请求字面回环 IP 和解析到回环的 `localhost` 都无法访问临时 HTTP 监听器。
3. 修复项目 Mihomo 生成器与独立模板：在生成规则首项加入 `IP-CIDR,198.18.0.0/15,REJECT,no-resolve`，早于额外规则集、私网 DIRECT 和 MATCH，并避免与基础模板重复。两种 Clash 格式和节点组订阅已重新发布；客户端需刷新订阅。
4. 确认 GTT 旧端口无人监听后，仅移除 43443/45443 TCP 的废弃 UFW 放行；当前端口 24443/24444 和 SSH 规则保留。
5. VMISS 先轮转、完整归档旧 journal，再下载并核对 SHA-256，随后设置 journal 256 MiB 上限、1 GiB 保留空间及 32 MiB 单文件上限，清理已归档 journal。归档在服务器和本机各保留一份，历史部署备份没有删除。当前 journal 约 184 MiB。
6. 核对三台原配置字节、节点参数、分享内容与组身份，订阅 URL、凭据、账期及流量修正记录保持不变；累积流量未倒退，采集与账期定时器 active。代理内核二进制摘要不变。

应用服务端出站防护时执行了必要的 sing-box 重启；GTT 在预检查和启动验证失败的自动恢复期间也发生重启。最终补充客户端模板和生产公钥的管理程序更新没有重启数据平面。不能将本轮描述为完全无中断。

## 验证与恢复

- WSL release 构建；工作区回归、Clippy、格式检查、真实 sing-box/Xray/Mihomo 配置及转发矩阵、NO_PROXY 防绕过、PTY 菜单验收通过。
- 最后新增 Fake-IP 回归覆盖 legacy/managed、节点组、4 模板、2 规则档及两种 Clash 格式；库测试 325 passed、1 ignored，固定 goldens 通过；真实 Mihomo 当前版及 1.18 的 32 项配置、实际 MRS 规则加载通过。
- 真实 Mihomo DNS 分配 Fake-IP 后，用非 hostname SOCKS5 配合该地址请求正常 HTTPS，返回 204；另一个无映射假地址在约 6 毫秒内命中规则 REJECT。正常映射未被兜底误拦，规则拒绝与核心自身的缺失映射拒绝已区分。
- 三台最终运行中订阅进程的可执行文件摘要与上述候选一致。配置和服务端覆写为 0600，项目服务保留 `NoNewPrivileges=yes`。
- 各台 root 专用备份均在 `/root/sbctl-deploy-20261117/`，包含原管理程序、代理内核、配置、服务单元、工件/代际指针及 UFW 配置。日志和详细验收输出保持私有。临时自动恢复定时器已撤销，计量定时器已恢复。
- VMISS journal 归档摘要：`6259190524fb86eea3770c97d8861001bf372e05c4c306b7ffcee148c6c9b450`。本机副本位于 ignored 私有验收目录，Windows ACL 限当前用户与 SYSTEM。

部署恢复与 journal 恢复是不同操作；不要直接全量解包历史 `/var/lib/sbctl` 覆盖当前计量和订阅凭据。私有恢复脚本只恢复部署相关文件，排除这些独立状态。

机器结果见 [脱敏验收记录](../.scratch/vps-deployment-20261117/results.json)。

## 尚存问题与边界

- **VMISS 当前出口的 TCP 路径仍异常。** 改配置或关闭 TLS 校验无法修复握手超时。现有 Hy2/TUIC 可继续使用；刷新其 HTTPS 订阅需使用可用代理或其他可达网络。恢复独立 TCP 直连需另换网络复测或由服务商处理路径/公网 IP。本次没有执行付费换 IP，也没有把绕行成功记为直连成功。
- **Xray 出站防护缺口。** 当前项目没有 Xray 服务端覆写入口，本次 sing-box 防护不覆盖 VMISS 两个 Xray 节点；见[后续事项](../.scratch/vps-deployment-20261117/issues/01-xray-egress-guard.md)。
- **客户端 Fake-IP 根因仍需有效配置确认。** 本次修复了模板的防漏兜底，未操作 Clash Party、安装外部 JS 或验证其后置合并行为。全局模式与管理员覆写可能改变兜底效果；服务端拒绝假地址不能恢复客户端已丢失的映射。
- **GTT 仍保持 legacy。** 为保留原 Reality handshake resolver，持久覆写中包含该字段；统一托管流程会拒绝它修改 `tls`。以后迁移或使用 `core apply` 前，需先将该 resolver 定制调整到受支持的顶层 DNS 设置。普通 legacy regenerate 已通过验证。
- 三台仍允许 root 密码 SSH；NTT/VMISS 的公钥认证当前关闭，UFW 当前 inactive。本次未改 SSH 认证策略或启用主机防火墙，以保留现有访问方式。后续可单独安排密钥登录和防火墙收敛。
- GTT 原有 `fwupd-refresh.service` 失败与项目服务无关，本次未更改；其余两台无失败单元。
- 未执行主机重启、生产破坏性故障注入、正式签名发布、跨架构或 GUI 导入验收。
