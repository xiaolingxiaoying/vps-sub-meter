# Rust 核心、运行时模板与 Xray 内核切换实施计划

Status: ready-for-agent
Created: 2026-11-09
Scope: 规划；尚未实施功能，不包含提交、推送、发布或生产 VPS 操作。

## 目标与首版范围

保留 Rust 编写的 sbctl 管理程序，部署无需新增 Python、Node.js 或 Go 工具链。实际代理连接由预编译的 sing-box 或 Xray 承担，已有 systemd、证书工具等系统依赖继续沿用。

1. 经常修改的客户端基础模板与策略通过 JSON/YAML 文件加载，执行应用命令后生效，无需重编译 sbctl。
2. 支持安装、更新和切换 sing-box / Xray；同一部署同一时刻仅运行一个托管数据平面内核。
3. Xray 首版实现 VLESS Reality 和 VMess WebSocket；其余协议必须经过固定版本官方二进制和真实连接验证才能加入支持矩阵。
4. 保留现有订阅格式、订阅凭据、VPS 流量统计与 accounting state 写入边界。
5. 切换失败恢复旧内核及旧配置；相同端口交接允许短暂中断，不承诺已有连接保持或零停机。

首版不做双内核并行、XHTTP 等新节点类型、通用脚本插件、自动监听文件热更新、Xray gRPC 连接管理，也不提供 Xray 客户端完整配置订阅。新增适配器需要更新 Rust 程序；数据文件只承担已实现能力的配置。

## 已知实现基础与风险

- `src/canonical.rs` 已提供 CanonicalNode，各客户端订阅应继续从同一模型生成，不从服务端原始 JSON 反向解析节点。
- `src/subscription/render/baseline.rs` 通过 include_str! 和 LazyLock 读取内置模板，并假设某些字段存在。外部数据必须改成显式加载、结构校验与错误返回。
- `src/subscription/template.rs` 的策略目录仍有固定组名与规则假设；仅把 include_str! 改成文件读取，不能实现可调整的代理组布局。
- `src/override_template.rs` 已有分层覆写；`src/subscription/artifacts.rs` 已有操作锁和事务，但服务端产物路径、活动配置与校验器绑定 sing-box。
- 客户端 full profile 当前会用服务端已安装 sing-box 选择版本。Xray 模式必须解除这层依赖，不能把 Xray 版本用于 sing-box 客户端版本选择。
- `src/lifecycle.rs`、`src/config.rs`、`src/update.rs`、`src/release.rs`、CLI 与安装器包含固定路径、服务账户、版本格式与发布清单假设，均须纳入迁移。
- Xray 官方 Hysteria 文档与此前查看的 v26.10.31 默认构建入口存在支持信息差异。首版不承诺 Hy2；01 必须核实选定发布工件，不能凭文档或名称猜测能力。

## 设计约定

### 内核与节点

DeploymentConfig 增加序列化枚举 `core = sing-box | xray`，缺失默认为 sing-box。保留既有协议凭据结构，切换共同支持的协议时不重新生成 UUID、Reality 密钥、端口、WS 路径或订阅凭据。

Rust 内部增加小范围内核适配边界：身份/版本识别、支持能力、服务端配置生成、配置检查、下载定位、运行参数与服务描述。先使用枚举和明确的适配实现，不引入动态插件系统。

能力按内核和经过验证的版本表达；未知版本不凭空开放协议。配置文件可以选择已实现的能力，不能自行声明一个未经验证的协议可用。

### 配置与模板

建议路径（实施时由 01 固化）：

```text
/etc/sbctl/templates/client/sing-box.json
/etc/sbctl/templates/client/mihomo.yaml
/etc/sbctl/templates/server/sing-box.json
/etc/sbctl/templates/server/xray.json
/etc/sbctl/overrides/sing-box-server.json    # 现有路径保留
/etc/sbctl/overrides/sing-box-server.d/
/etc/sbctl/overrides/xray-server.json
/etc/sbctl/overrides/xray-server.d/
```

客户端覆写的现有路径不变。服务器默认仍由类型化 Rust 生成器负责节点、凭据、监听器和证书路径；可选服务端模板只承载已定义的基础设置，使用与既有服务端覆写一致的受保护字段边界，不实现整份未经约束的原始配置直通。

每次生成在操作锁内加载一份有大小限制的模板快照，后续生成和校验均使用这份快照。外部模板优先，缺失时使用内置默认；已存在但格式错误、结构不兼容或引用错误时拒绝应用，保留已发布产物，不静默回退。程序升级不覆盖管理员模板。

客户端处理顺序：基础模板 → Rust 注入真实节点和显式配置、处理组布局及版本兼容 → 现有管理员覆写 → 引用/结构校验 → 可用的对应客户端真核校验 → 发布。裸节点和 URI 继续遵守现有不应用客户端模板覆写的契约。

配置/模板 schema 版本由 sbctl 的加载契约管理，不把管理字段直接发给内核。无需引入字符串拼接式 JSON 占位；通过结构化文档插入节点与凭据。第一阶段支持 DNS、TUN、路由、规则来源、已有组类型的布局与测速设置，保留程序对不兼容字段的检查。

### 服务端与客户端校验

sing-box 服务端使用 sing-box check，Xray 服务端使用目标 Xray 的 run -test。客户端 sing-box/Mihomo 配置仅能由各自的校验器检查。

客户端目标版本由现有版本注册表、显式版本订阅路径和明确的 full-profile 默认策略决定。Xray 模式在没有额外客户端校验器时仍须能运行：执行结构/引用/已知兼容性校验，明确报告未进行真核校验；发布门禁在 CI 中使用固定版本客户端内核覆盖。可选校验二进制无需作为服务运行。

旧 sing-box 部署应保持历史 full-profile 选择行为，除非迁移 ADR 明确记录变化；新增 Xray 默认策略必须在 01 固化并测试。

### 切换事务与恢复

先以只读 plan 计算影响，再下载/验证并暂存目标内核、加载模板、生成配置与订阅、运行目标内核检查。检查失败不得停止旧服务。

预检必须处理不支持的协议、Hy2 跳跃规则、服务端覆写、Clash API 设置、证书读取权限、托管服务归属及外部端口占用。不支持项默认阻止切换；交互中可明确选择停用受影响协议，非交互须显式指定停用列表。不删除相关凭据与用户文件，切回时可重新启用。所有受影响设置在 plan 中列出。

进入激活阶段前持久化事务记录与私有快照。停止旧服务后切换活动配置和 unit，启动目标服务并检查稳定性/监听器。通过后发布一致的订阅与部署状态，再标记事务完成。失败恢复程序、unit、配置、订阅、权限及必要的防火墙状态并启动旧服务。订阅 HTTP 服务在数据平面切换期间继续运行；其正常读取失败处理规则保持不变。

多文件逐个原子替换不等于整个目录事务。实现必须明确提交点和中间状态，覆盖服务重启失败及进程中断后的恢复；不得只依赖内存中的回滚快照。手动 rollback 恢复最近一次成功切换前的部署，使用现有操作锁并检查快照状态，避免覆盖后续操作；具体保留/失效规则由 06 实现并记录。

目标内核必须以非 root 身份运行，只管理 sbctl 拥有的路径和 unit，不接管现有第三方 sing-box/Xray 服务。已有 sing-box 服务路径尽量保留；Xray 采用独立路径/unit，激活事务保证不会同时启用两者。证书续期钩子指向当前活动内核，正确授予读取权限。

VPS traffic 与 accounting period 不随切换重置。Xray 首版提供版本、PID、内存、运行状态、日志；Clash 专属连接管理明确显示不可用，保持该配置以便切回，不伪造对应功能。

### 下载与发布

只使用 SagerNet / XTLS 官方发布工件，固定版本、校验摘要及可执行文件身份，不在本项目重新托管内核。区别 GitHub 摘要校验与项目签名清单提供的认证边界。

扩展发布清单必须版本化；旧 v1 清单保留原签名载荷验证规则，不能因增加默认字段改变历史签名内容。新清单表达独立内核版本、架构、摘要与兼容范围，同时更新安装器、签名脚本和 release workflow。sbctl 更新不得隐式切换内核，内核更新不得旋转凭据。

## 实施顺序与任务

| 阶段 | 任务 | 前置任务 | 可验收结果 |
| --- | --- | --- | --- |
| 1 | [01 能力验证与契约](issues/01-capabilities-and-contracts.md) | 无 | 固定版本、协议矩阵、迁移与模板契约明确 |
| 2 | [02 内核抽象与校验解耦](issues/02-core-adapters.md) | 01 | sing-box 行为保持，服务端/客户端校验边界分离 |
| 3 | [03 运行时模板](issues/03-runtime-templates.md) | 01、02 | 改模板即可重新生成；失败保留旧产物 |
| 4 | [04 Xray 配置适配](issues/04-xray-rendering.md) | 01、02、03 | 两种协议通过目标内核检查与客户端连接 |
| 5 | [05 下载与发布清单](issues/05-downloads-and-manifests.md) | 01、02 | 两种内核独立下载验证，旧清单兼容 |
| 6 | [06 切换事务与恢复](issues/06-switch-transactions.md) | 03、04、05 | 双向切换，故障和中断均可恢复 |
| 7 | [07 菜单与兼容入口](issues/07-cli-and-menu.md) | 03、06 | 预览影响、明确切换、查看状态和恢复 |
| 8 | [08 验收与发布准备](issues/08-validation-and-docs.md) | 01–07 | Linux systemd、真核互通、升级和回滚证据 |

03 完成后即可交付“不重编译改模板”，不必等待 Xray 全部完成。05 与 03/04 在契约稳定后没有实现依赖，可分别推进；表格采用便于逐步验收的默认顺序，不要求多代理执行。

## 验收与发布门禁

- 既有部署缺少 core 字段仍能读取、重生成和更新，未选外部模板时保持现有输出契约。
- 仅改客户端模板不重启数据平面；外部模板非法、缺引用、版本不兼容时不发布坏订阅。
- sing-box ↔ Xray 两方向验证 VLESS Reality、VMess WS 的真实连接；共同字段未变化时凭据、端口、订阅入口不变。
- 目标内核不支持的协议、覆写和功能在停旧服务前被识别；不能无提示删除节点。
- 坏摘要、错误架构/版本、非法配置、端口冲突、权限不足、启动退出/重启循环、发布写入失败及关键提交点中断均有恢复证据。
- VPS traffic、周期、流量修正与订阅凭据宽限状态不因切换丢失或重置。
- 在真实 Debian/Ubuntu systemd 环境验证非 root 运行、证书续期重启、权限、卸载归属和失败恢复；WSL 不能代替生产环境验收。
- 使用现有 CI 基线：fmt、clippy、workspace/test-signing 测试、release_trust、菜单/安装验收、文件权限与受管路径检查，并补齐 Xray 与模板用例。Python/Node 只允许沿用开发/CI 测试工具，不成为新增生产运行依赖。
- 发布准备包含架构契约、用户文档与发布门禁更新；实际发布不属于本次计划的执行授权。

## ADR 与领域文档调整

实施时新增多内核与运行时模板 ADR，明确修订而非静默违背以下既有决策：ADR-0007（校验/回滚）、0010（签名清单）、0012（非 root）、0022（编译期模板）、0029（覆写边界）、0031（Clash 观测）、0037（证书发布）、0038（官方内核来源/Hy2 跳跃）。更新 CONTEXT.md 的服务端内核、模板及支持协议定义，保留单管理员产品边界。

## 待 01 核实的来源

- [Xray 官方发布](https://github.com/XTLS/Xray-core/releases)
- [Xray 入站协议](https://xtls.github.io/config/inbounds/)
- [Reality](https://xtls.github.io/config/transports/reality.html)
- [Hysteria](https://xtls.github.io/config/inbounds/hysteria.html)
- [v26.10.31 构建入口](https://github.com/XTLS/Xray-core/blob/v26.10.31/main/distro/all/all.go)
- [Xray run / test 实现](https://github.com/XTLS/Xray-core/blob/v26.10.31/main/run.go)

这些是研究入口，不代表已完成所选工件的兼容性验收。
