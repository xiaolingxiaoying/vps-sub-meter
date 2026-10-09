# 订阅 TLS 超时排查

Direct HTTPS 服务对 TLS 握手设置 10 秒上限，对接受连接到 HTTP 结束设置 30 秒总上限，HTTP 请求头最多等待 5 秒。32 个并发槽位会在连接结束或超时后回收。无界等待 ClientHello 曾允许慢连接长期占满槽位；这一代码缺陷可以造成正常客户端连接失败，但不能据此确定某台 VPS 的全部超时原因。

证书检查每秒进行一次，文件读取和解析在阻塞任务中完成。更新失败时保留尚未过期的旧证书，同一文件状态失败后等待 30 秒再尝试。已经过期的证书不会继续接受新连接。每分钟的 `Direct HTTPS last 60s` 日志区分握手超时、握手错误、连接超时、容量拒绝与证书不可用，不记录订阅路径或凭据。

在 VPS 执行：

```sh
sudo sbctl system diagnose-subscription --json
sudo journalctl -u sbctl.service --since '15 minutes ago' --no-pager
sudo systemctl status sbctl.service sbctl-http.socket sing-box.service
```

诊断输出包含部署模式、域名、端口、证书状态、systemd 状态与重启次数、DNS A/AAAA，以及 Direct 模式使用域名 SNI 的 loopback TLS GET `/`。`public_port` 是订阅公开入口；ExternalProxy 的公开 HTTPS 端口为 443，`listener.target` 则是本地 HTTP 上游端口。该 TLS 探测使用系统 CA 和主机名校验，不需要订阅凭据。`curl_unavailable` 表示需要安装发行版 curl。非真实根目录的测试部署会跳过主机探测。

从发生故障的客户端网络使用无凭据根路径分别检查 IPv4 和 IPv6：

```sh
curl -4 -v --connect-timeout 10 --max-time 30 https://example.com/
curl -6 -v --connect-timeout 10 --max-time 30 https://example.com/
```

根路径返回 404 等 HTTP 状态仍能证明 TCP/TLS 已完成。不要把真实订阅 URL 放进共享日志。结合结果处理：

| 证据 | 处理方向 |
| --- | --- |
| loopback TLS 成功，公网 TCP 失败 | 检查 VPS 安全组、防火墙、运营商路由和入口端口；Proxy 模式另查反向代理 |
| IPv4 成功而 IPv6 失败，域名存在 AAAA | 修复 IPv6 地址、路由与监听，或移除错误 AAAA；不要盲目禁止全部 IPv6 |
| certificate_validation_failed | 检查证书链、域名、有效期和系统时间，修复证书；保持验证开启 |
| handshake_timeout / capacity_rejected 增长 | 检查慢连接、扫描、丢包与服务负载；确认运行的是含限时修复的新版本 |
| 服务重启数增长 | 检查崩溃、OOM、资源限制与证书更新日志 |
| 仅部分第三方客户端失败 | 对比客户端代理路径、DNS、时间和 TLS 错误；订阅请求可能经过当前代理/TUN |

本轮没有真实 VPS 故障日志，因此以上是取证步骤和已修复代码缺陷，尚不是线上修复验收。

## 节点提示“非预期的 SSL 证书”

节点的 TLS 与下载订阅的 HTTPS 是两条独立连接。默认 `certificate_mode = "self-signed"` 时，VMess WebSocket、Hysteria2、TUIC、AnyTLS 使用程序生成的自签证书；客户端导入节点时必须保留对应的证书策略。VLESS Reality 使用自己的密钥认证，不适用这组参数。

旧版 VMess Base64 JSON 分享链接遗漏了证书策略字段，即使 sing-box JSON 和 Clash YAML 已正确生成，URI、Base64 URI、Shadowrocket 订阅和节点二维码导入后仍可能拒绝自签证书。修复后，自签模式的 VMess JSON 带有 `insecure: "1"` 和 `allowInsecure: "1"`；其他 TLS 协议链接同时输出 `insecure=1` 与 `allowInsecure=1`，TUIC 另带 `allow_insecure=1`，兼容采用不同参数名的导入器。域名证书模式保持证书验证开启。

参数名参考 [v2rayN 的 VMess 导入导出代码](https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Handler/Fmt/VmessFmt.cs) 和 [sing-box-yg 的分享链接生成代码](https://github.com/yonggekkk/sing-box-yg/blob/main/sb.sh)。Shadowrocket 没有公开完整的导入规范，仍需用实际客户端验证其导入结果。

在 VPS 更新 sbctl 后执行 `sudo sbctl regenerate`，再刷新客户端订阅；手动导入的旧节点需要重新导入更新后的分享链接或二维码。仅重启服务不会重新生成订阅。如果报错来自订阅 HTTPS 下载本身，或使用域名证书模式，继续检查证书域名、信任链、有效期和系统时间，不应通过关闭验证处理。
