# 订阅链接与二维码审查（2026-10-19）

测试目标：用户指定的 Ubuntu 22.04 VPS，现网 sbctl 0.0.13、sing-box 1.14.2，Direct HTTPS，五协议均启用，协议节点使用自签证书。实际订阅 URL、订阅凭据、节点凭据和 SSH 口令均不写入本报告。

## 发现与修复

1. **P1：旧版 Clash 配置无法导入。** 现网 `clash-1.18.yaml` 包含 `type: anytls`，用官方 mihomo 1.18.10 验证得到 `proxy 4: unsupport proxy type: anytls`，整份配置加载失败。修复时先过滤 AnyTLS，再生成代理组，避免留下悬空节点引用。原测试只用新内核检查两个配置，未覆盖真实旧版解析器；现已增加独立的 `MIHOMO_LEGACY_BIN` 和 CI 下载步骤。依据：[mihomo 1.18.10 官方解析器](https://github.com/MetaCubeX/mihomo/blob/v1.18.10/adapter/parser.go)。

2. **P2：终端二维码缺少留白且跨行保留背景色。** 原实现没有四个模块宽的白色留边，也没有在每行换行前重置终端样式。已增加留边、初始样式重置和每行末尾重置。原版在独立解码器中仍可解出，不能断言它是所有手机扫码失败的唯一原因；修复消除了这两个影响终端呈现和识别稳定性的缺陷。四模块留边要求见 [DENSO WAVE 二维码说明](https://www.qrcode.com/en/howto/code.html/index.html)。

3. **P2：仅启用 AnyTLS 时公布不存在的旧版订阅。** 原先 sing-box 1.10/1.11 工件被跳过，但 CLI、总览页和二维码仍公布对应链接。现在 CLI、菜单、邮件和总览页共用部署可用格式列表；仅 AnyTLS 时排除 sing-box 1.10、1.11 和 mihomo 1.18。显式请求这些订阅及其二维码返回 404，即使磁盘残留旧文件也不会重新开放。

4. **P2：通用分享 URI 未编码密码保留字符。** Hysteria2、TUIC、AnyTLS 的通用 URI 直接拼接密码，配置包含 `@`、`?`、`#`、`/`、`%` 等字符时会改变 URI 结构。已统一编码，并以 IPv4、IPv6 和包含中文的密码回归验证。VLESS SNI 也使用相同编码函数。默认自动生成密码的历史 URI/Base64/Shadowrocket 快照保持通过。依据：[AnyTLS 官方 URI 格式](https://github.com/anytls/anytls-go/blob/main/docs/uri_scheme.md)。

页面同时更正了“全版本兼容”“系统相机一键导入”等过度承诺：`sing-box.json` 是节点配置片段；订阅二维码携带 URL，应使用客户端的订阅扫码入口。

## 逐条验证

下列路径均省略 `/sub/<订阅凭据>/`。12 个地址全部通过公网 HTTPS GET，返回字节与服务器工件一致，具有流量信息头；对应 `/qr/<路径>` 全部返回 SVG，解码结果与原订阅 URL 完全相同。网页内嵌的 12 个二维码也全部匹配；SVG 按原始尺寸及页面使用的 180px 尺寸均能解码。

| 路径 | 现网节点数 | 现网实际内核验证 | 修复后的行为 |
| --- | ---: | --- | --- |
| `sing-box.json` | 5 | 1.14.1 check 通过；配置片段 | 保持节点数据 |
| `sing-box-1.10.json` | 4 | 1.10.7 check 通过 | 不含 AnyTLS；仅 AnyTLS 时隐藏 |
| `sing-box-1.11.json` | 4 | 1.11.15 check 通过 | 不含 AnyTLS；仅 AnyTLS 时隐藏 |
| `sing-box-1.12.json` | 5 | 1.12.25 check 通过 | 保持 |
| `sing-box-1.13.json` | 5 | 1.13.21 check 通过 | 保持 |
| `sing-box-1.14.json` | 5 | 1.14.1 check 通过 | 保持 |
| `sing-box-full.json` | 5 | 1.14.1 check 通过 | 保持；客户端须匹配支持版本 |
| `clash.yaml` | 5 | 现行 mihomo 检查通过 | 保持 |
| `clash-1.18.yaml` | 5 | **1.18.10 拒绝 AnyTLS** | 过滤为 4 节点；仅 AnyTLS 时隐藏 |
| `uri` | 5 | 明文 URI 结构、端点、凭据检查通过 | 修复保留字符编码 |
| `uri.txt` | 5 | Base64 解码及节点参数检查通过 | 同上 |
| `shadowrocket.txt` | 5 | Base64 解码及节点参数检查通过 | 保持专用格式 |

各 JSON/YAML/URI 的节点地址、端口和凭据相互一致；JSON 的 TLS 设置也一致。三个 URI 工件的 VMess 均包含 `insecure: "1"` 和 `allowInsecure: "1"`，符合此 VPS 的自签节点证书模式，未发现旧版遗漏字段的问题。

未知版本、未知二维码格式、附带 query 参数、附加路径均返回 404。

## 实际代理连通性

从独立 WSL 客户端使用下载的真实节点参数，逐个启动 sing-box 并经 SOCKS 请求 `https://www.gstatic.com/generate_204`：

| 协议 | 结果 |
| --- | --- |
| VLESS Reality | HTTP 204 |
| VMess WebSocket | HTTP 204 |
| Hysteria2 | HTTP 204 |
| TUIC | HTTP 204 |
| AnyTLS | HTTP 204 |

这验证了五协议的实际连接、认证和 HTTPS 请求转发；未替代 Shadowrocket 等客户端 GUI 的真实扫码导入测试。

## 审查阶段验证及部署边界（发布前）

- `scripts/dev/wsl-gate.sh`：fmt、clippy 通过；467 项测试通过，0 失败。默认忽略的真实内核用例另行运行。
- `scripts/dev/wsl-real-cores.sh`：五个 sing-box minor 的配置、示例覆写及服务端配置验证通过；现行 mihomo 和官方 1.18.10 分别检查自身格式，包含四模板、两种规则方案及示例覆写的组合。
- 在 VPS 临时隔离目录中，候选 sbctl 使用现网配置、覆写文件和本地规则副本重新生成工件，由现网 sing-box 1.14.2 校验服务端配置。候选的 12 个终端订阅二维码和 5 个原生节点二维码全部经独立 ZXing 解码器解出；订阅 URL、四模块留白和每行样式重置均验证通过。
- 除修复后的旧版 Clash 外，其余 11 个订阅工件与现网逐字节一致，自定义分流规则保持原样。包含现网自定义规则的旧版 Clash 候选工件也通过官方 mihomo 1.18.10 检查。
- 验证结束后已删除 VPS 临时目录，并确认现网 sbctl、部署配置和运行配置的 SHA-256，以及两个服务的主进程 PID 均保持不变。
- 修复保留在本地工作区；未替换现网 sbctl，未重新生成现网订阅，未重启线上服务。因此现网旧版 Clash 文件仍需在正式更新后执行 `sbctl regenerate` 才会修复。

含敏感节点信息的临时验证工件仅保存在已忽略的 `.scratch/research/subscription-audit/`，不进入提交。
