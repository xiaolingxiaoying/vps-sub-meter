# 订阅链接详细指南

sbctl 在同一份节点模型上生成多种订阅格式。所有链接都在 `/sub/<凭据>/...` 路径内，凭据只放在 path 中，拒绝 query 参数；每条链接都有对应的二维码链接（`qr/<格式>`），`index` 总览页一次性列出全部链接、标注与二维码。

| 链接 | 内容 | 适用客户端 |
| --- | --- | --- |
| `/sub/<cred>/sing-box.json` | 仅 outbounds 节点片段，需合并到客户端配置 | 含 AnyTLS 时要求 sing-box 1.12+ |
| `/sub/<cred>/sing-box-full.json` | 最新稳定版完整客户端配置 | sing-box 最新稳定版（V2rayN 6.6+ 亦可导入） |
| `/sub/<cred>/sing-box-1.10.json` | 1.10.x 适配完整配置 | sing-box ≥1.10.0, <1.11.0（无 AnyTLS 节点） |
| `/sub/<cred>/sing-box-1.11.json` | 1.11.x 适配完整配置 | sing-box ≥1.11.0, <1.12.0（无 AnyTLS 节点） |
| `/sub/<cred>/sing-box-1.12.json` | 1.12.x 适配完整配置 | sing-box ≥1.12.0, <1.13.0 |
| `/sub/<cred>/sing-box-1.13.json` | 1.13.x 适配完整配置 | sing-box ≥1.13.0, <1.14.0 |
| `/sub/<cred>/sing-box-1.14.json` | 1.14.x 适配完整配置 | sing-box ≥1.14.0 |
| `/sub/<cred>/clash.yaml` | 现行稳定版 mihomo 配置 | mihomo 现行稳定版（rule-set 分流） |
| `/sub/<cred>/clash-1.18.yaml` | 旧版兼容配置，不含 AnyTLS | mihomo 1.18.x |
| `/sub/<cred>/uri` | 明文分享 URI（每行一条） | 通用 |
| `/sub/<cred>/uri.txt` | 明文 URI 整体 Base64 | V2rayN 等 |
| `/sub/<cred>/shadowrocket.txt` | Shadowrocket 适配 Base64 URI | Shadowrocket (iOS) |
| `/sub/<cred>/qr/<格式>` | 对应链接的二维码（SVG） | 手机扫码导入 |
| `/sub/<cred>/index` | 中文总览页（按客户端速查 + 全链接 + 标注 + 二维码 + 导入步骤） | 浏览器 |

## 按客户端选择（主流客户端速查）

| 客户端 | 推荐订阅链接 | 说明 |
| --- | --- | --- |
| Clash Party | `clash.yaml` | mihomo 内核订阅，导入后自动更新节点 |
| Clash Verge | `clash.yaml` | mihomo 内核；内置内核较旧时改用 `clash-1.18.yaml` |
| sing-box | `sing-box-full.json`；按客户端实际内核版本选 `sing-box-<版本>.json` | 1.10/1.11 不支持 AnyTLS 节点（1.12.0 才加入） |
| V2rayN | `uri.txt`（Base64 URI，默认内核） | 6.6+ 可直接导入 `sing-box-full.json`（内置 sing-box 内核） |
| Shadowrocket | `shadowrocket.txt` | 五协议均支持，需 ≥ 对应协议最低版本（见下文） |

`sbctl sub` 会先输出以上按客户端速查，再输出完整矩阵；`index` 总览页顶部同样提供该速查表。

管理员可以在服务器终端运行 `sbctl node --links` 查看各协议原生分享链接；链接含 Proxy credential，因此默认的 `sbctl node` 不会显示。可以用 `--protocol vless-reality` 等协议名筛选，`--qr` 会在终端渲染对应链接的二维码（需要同时指定 `--links`）。旧参数 `--uri` 仍可作为 `--links` 的兼容别名。订阅总览页中的节点链接默认折叠，展开后才显示；不要截图或转发这些链接。

## 完整客户端配置包含什么

`sing-box-full.json`（及各版本文件）与 `sing-box.json` 的区别：

- `log`：info + 时间戳；
- `dns`：fake-ip 模式（`client_dns_mode` 可切 redir-host）；私有域名走系统 DNS，国内、连通性检测与 NTP 域名走国内 DoH（223.5.5.5），其他查询走境外 DoH（1.1.1.1，detour `代理` 组）；
- `inbounds`：tun（auto_route/strict_route/mixed 栈，MTU 1400，无示例网段排除）及回环 mixed/SOCKS/HTTP 监听（7890/7891/7892）；
- `outbounds`：`代理`（selector）+ `自动选择`（url-test，探测 URL 可配置）+ 五协议节点 + `DIRECT`；
- `route`：AI 域名（chatgpt/openai/x.com 等）优先走选择组；geosite-cn / geoip-cn rule-set 直连（standard 档）；私有地址直连；`default_domain_resolver` 指向直连 DNS；
- `experimental`：clash_api（127.0.0.1:9090，供 SFA/SFW 面板与 sbtui 使用）+ cache_file（记住选择与 fake-ip 映射）。

## sing-box 版本差异（1.10 → 1.14）

差异以官方 changelog 研究为准（`docs/research/sing-box-client-version-differences.md`）：

| 版本 | DNS servers | 特殊出站/ sniff 字段 | 客户端兼容性说明 |
| --- | --- | --- | --- |
| 1.10.x | legacy 字符串格式 + 顶层 fakeip | 入站 `sniff` 字段 + 特殊 `dns` outbound | **不含 AnyTLS 节点**；无 `store_dns`；无 `default_domain_resolver`（用 `outbound: any` DNS 规则） |
| 1.11.x | legacy 字符串格式 + 顶层 fakeip | 规则动作（`action: sniff` / `hijack-dns`） | **不含 AnyTLS 节点**；无 `store_dns` |
| 1.12.x | 新对象格式（legacy 告警） | 仍可用（弃用） | geoip/geosite 字段已移除，tun 用 `address`；无 `store_dns` |
| 1.13.x | 新对象格式 | 已移除，改规则动作 | WireGuard 出站移除（本配置未用）；无 `store_dns` |
| 1.14.x | legacy 格式移除 | 已移除 | DNS 规则 `outbound` 项移除；`cache_file.store_dns` 可用；与服务端运行的最新稳定版一致 |

生成策略：每个版本 profile 只带该版本内核能接受的字段——1.10/1.11 用 legacy DNS 与顶层 `dns.fakeip`，1.10 的路由规则不用动作式写法，任何版本都不会收到它不认识的字段。`store_dns` 仅 1.14 工件携带。AnyTLS 节点只在 1.12+ 工件中出现；**当部署只启用了 AnyTLS 时，1.10/1.11 工件不会生成**（生成时打印警告，其余格式不受影响）。

仅启用 AnyTLS 时，也不生成 `clash-1.18.yaml`。CLI、菜单、邮件和总览页会隐藏这三个无兼容节点的格式；手动请求对应订阅或二维码返回 404，避免把缺失工件误报为服务故障。

## mihomo 差异（1.18 → 1.19）

- `clash.yaml` 使用四个 `review-*` rule-providers（远程 `.mrs`，`@meta` 分支的 geosite/geoip cn/private）。标准模板有 `代理`、`自动选择` 两个组；精简模板只输出 `代理`，成员为真实节点与 `DIRECT`。GLOBAL 由内核提供。
- `clash-1.18.yaml` 通过内置域名/IP 列表保留直连分流，不引用远程规则集或自动下载地理数据库。
- mihomo 1.18 不认识 `type: anytls`，会拒绝加载包含该类型的整份配置。旧版工件在生成代理组前排除 AnyTLS 节点；真实内核测试分别使用现行版本和 1.18.10，不能仅用新内核验证旧版兼容文件。

## 二维码使用

订阅二维码编码的是对应订阅 URL。请在客户端的“添加订阅 / 扫码”入口使用；系统相机可能只打开浏览器下载，不能保证自动导入。节点分享二维码编码的则是单个节点 URI，二者用途不同。

终端二维码包含四个模块宽的白色留边，每行在换行前恢复终端颜色。请保留全部留边、使用等宽字体，并确保窗口足够宽以免二维码自动折行；密集节点二维码可扩大终端或使用订阅总览页的 SVG。
- 两份工件都保留参考模板的 HTTP/TLS/QUIC `sniff` 映射、端口和跳过列表，明确设置 `override-destination: false`；TUN 严格路由与 UDP/TCP 53 DNS 劫持开启。完整基础设置见 [内置模板说明](../src/subscription/templates/README.md)。
- rule-providers 的本地 `path` 使用相对路径 `./ruleset/*.mrs`，位于客户端 workdir 内。

## Shadowrocket 适配说明

`shadowrocket.txt` 是 Base64 URI 列表，与通用 `uri.txt` 的区别（依据 `docs/research/` §6）：

- 密码与 SNI 一律百分号编码（Shadowrocket 2.2.44 修复 URI 密码解码，特殊字符必须编码到达）；
- TUIC 增加 `udp_relay_mode=native`；
- AnyTLS 遵循官方 anytls-go URI 规范（路径斜杠、`insecure`、`sni`，去掉非标准 `security` 参数）；
- 五协议均支持：VLESS Reality ≥2.2.16、TUIC ≥2.2.12、Hysteria2 ≥2.2.35、AnyTLS ≥2.2.64。

导入：Shadowrocket → 首页右上扫码或「添加配置」粘贴链接 → 打开「自动更新」。

## 覆写模板（服务端统一分发）

不想在每个客户端里手工维护覆写时，把规则放到服务器上。修改这些文件无需重新编译 Rust；[不重编译的覆写操作指南](client-overrides.md) 提供可直接使用的 YAML/JSON 示例、应用及恢复步骤。

常用命令：

```bash
sbctl config override show      # 查看路径与合并语义
sbctl config override edit clash     # $EDITOR 编辑（默认给出示例）
sbctl config override edit server --layer 10-dns.json # 编辑服务端分层覆写
sbctl config override validate  # 校验服务端与客户端 sing-box 配置
sbctl config override clear     # 清除客户端覆写（基础文件与 .d 层）
sbctl config override clear server # 仅清除服务端覆写
sbctl config override clear all    # 清除所有覆写
```

合并语义（ADR-0021/0029）：对象递归合并；默认数组整体替换；**键名为 `rules` 的数组前插**到生成规则之前，每个文件可用 `rules_mode` 选择 `prepend`、`append` 或 `replace`。sing-box 的 `outbounds` 按 `tag` 合并，Clash 的代理、策略组和规则提供者按 `name` 合并。客户端目标影响 sing-box-full 与各版本文件、clash.yaml、clash-1.18.yaml；服务端目标修改本机运行的 `sing-box-server.json`。客户端覆写不改变 `sing-box.json` 与 URI 格式。

## 客户端模板配置

`sbctl menu` →「订阅与客户端」可切换精简 / 标准模板；其他客户端设置通过同页「高级客户端设置」调整：

每项都会显示当前值作为默认答案；保存前预览会列出模板、DNS 模式、规则档位、规则镜像和测速 URL，
订阅凭据保持脱敏，URL 中的认证信息、查询参数和片段也会隐藏。URL 选项仅接受 `http://` 或 `https://` 地址。

- `client_dns_mode`：fake-ip（默认）/ redir-host
- `client_template`：**compact / standard** —— 编译期内置的客户端结构（ADR-0022），不代表磁盘上的管理员模板文件。
  - `compact`：单一手动 `代理` 组，只含真实节点与 `DIRECT`，不提供自动选择组。
  - `standard`（新部署默认）：手动 `代理` 组含真实节点、`自动选择` 与 `DIRECT`，并另有只含真实节点的测速组。
  - 两种档位共享审阅过的 DNS、TUN、嗅探、规则顺序和规则集策略。`global` / `split` 保留序列化兼容，也可通过非交互 `sbctl config template global|split` 显式使用；交互选择器只显示精简与标准。
- `client_rule_profile`：standard（远程 rule-set，默认）/ minimal（全部内置规则，不访问规则 CDN）。
  `minimal` **只改获取方式，不改分流结果**：每个规则集支撑的规则都带一份编译期内联孪生，
  所以在 `minimal` 下 CN/私有目标仍然直连，而不是悄悄落到代理组。
  Clash 侧也一样：`minimal` 不再输出 `GEOIP,LAN` / `GEOIP,CN`——那两个代码由 mihomo 自己的
  geo 数据库回答，而这个数据库在文件缺失时同样是开机从 GitHub 下载的。代价要写清楚：内联的是
  粗粒度的 /10–/11 CN 网段与一份常用 CN 域名后缀，覆盖率低于完整数据库；
  能接受这个代价、又要零下载的客户端用 `minimal`，否则用默认 `standard`。
- `client_rule_set_base_url`：默认 `https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat`（`@sing`/`@meta` 分支由 sbctl 附加；换镜像只改这一处）。
  注意 `geoip/lan` 在该镜像上解析不到，因此 LAN 一直是内置列表而不是规则集 URL。
- `client_latency_probe_url`：默认 `https://www.gstatic.com/generate_204`（选择组含 DIRECT，探测必须国内可达）

修改后运行 `sbctl regenerate` 重新生成并生效（`sbctl restart` 只做 `sing-box check` 与重启服务，不重新生成工件）。真实内核校验命令为 `cargo test --test version_profiles -- --ignored`，覆盖 1.10–1.14、四种可渲染模板和两种规则档位；Mihomo 校验命令为 `cargo test --test clash_mihomo -- --ignored`。这些忽略测试需要通过环境变量提供实际上游二进制；本段说明校验入口，不表示当前环境已经运行或通过。

## 安全边界

- 凭据只走 URL path；query 参数、错误凭据、未知路径一律 404。
- 按源 IP 限流：同一地址可在瞬间花掉 60 次请求的突发额度，之后每秒只恢复一次；超出时返回 `429` + `Retry-After`。计费和判定都发生在读 URL 之前，所以**被限流时真凭据与错凭据的响应完全相同**（都带同样的头、空正文、不回显凭据），探测者无法用"是否 429"来反查某个订阅是否存在。ACME 挑战路径不受此限：它由 Let's Encrypt 的服务器发起，掐断它等于让证书续期失败。
  - **external-proxy 模式下这层限流被关闭**：该模式里对端地址永远是本机反代（`127.0.0.1`），
    所有真实用户会共用同一个令牌桶，等于自我制造宕机；而真实客户端地址在转发头里，
    本服务**刻意不信任** `X-Forwarded-For`——可伪造的转发头会让限流反过来变成
    "这个订阅是否存在"的预言机，也让人可以嫁祸别人。每客户端限额请配在前置代理上。
  - 读不到对端地址时放过（fail open）：fail closed 会在任何平台意外隐藏 peer 时
    把所有订阅一起停掉。这层是公平/滥用下限，不是安全边界。
  - 反代只在 `external-proxy` 下被建模。若你把 `direct`/`ip-fallback` 的 sbctl 也挡在本机
    nginx/caddy 后面（模式没改），对端地址就变成反代自己（回环，或宿主自己的 IP），
    **全体访客会共用一个令牌桶**——表现为随机 429，而不是攻击。这种拓扑请改模式，
    或把每客户端限额配在反代上。
- 所有响应带 `Cache-Control: no-store`；订阅凭据泄露时执行 `sbctl credential rotate` 全部作废。
- IP fallback 模式为明文 HTTP，仅建议无域名时临时使用。

## `subscription-userinfo` 的字段含义

订阅响应带一个 `subscription-userinfo` 头，键序是对外契约（客户端按名字解析、按此顺序展示，
新键只追加在末尾），由测试用**整串等式**锁死：

```text
upload=<已用上行字节>; download=<已用下行字节>; total=<额度>; expire=<下次重置的 Unix 秒>; profile-update-interval=24
```

- **方向按客户端视角**：`upload=` 是客户端发出的字节，也就是 VPS 网卡的 `rx_bytes`；
  `download=` 是 VPS 发出的字节，即 `tx_bytes`。统计源永远是网卡的 rx/tx（`src/traffic.rs`），
  但表头标签站在订阅用户那一边——v2rayN、Clash Verge、Shadowrocket 都把这两个键显示成
  "我上传/我下载了多少"。早先的实现按服务端视角打标签，于是下载为主的月份在客户端里显示成巨额"上传"。
  （`docs/implementation-plan.md` 里那行 `download=RX、upload=TX` 是当年有意的写法，现已按消费方语义纠正。）
- `total=` 只在配置了月度额度时发送，值是额度本身；无限额时省略 `total`，避免客户端把当前已用量误显示成总额度。`upload` / `download` 与 `expire` 仍照常发送。
- `expire=` 承担"刷新/重置日期"语义：本项目没有账号到期概念，它就是账期的下次重置时刻。
  因此**不设** `refresh=` 键（各客户端渲染重置日期用的就是 `expire`）。
- `profile-update-interval=24` 是对客户端的**策略声明**（"一天拉一次就够"），不描述服务端行为：
  仓库里没有"订阅自动刷新间隔"这一配置项。
- 账期状态读不出来时，宁可不发这个头也不发伪造值：响应仍是 200、正文正常，只是少了这个头。
- `sbctl traffic` / `status` 打印的 `received:` / `transmitted:` 是**服务端网卡视角**的原始计数，
  与上面这个头是两个方向，不要互相"对齐"。
