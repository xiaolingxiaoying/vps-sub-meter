# P2：远程规则集混用 SRS 与 MRS

Status: resolved
Priority: P2

位置：`src/subscription/render/clash.rs:217`、`src/subscription/render/singbox.rs:689`。

`rule-set add` 接受 .srs 或 .mrs，但两个渲染器都无条件使用同一个 URL。注册 .srs 后，clash.yaml 把它声明为 `format: mrs`；注册 .mrs 后，sing-box 全配置把它声明为自己的 binary 规则集。两者不能互读，订阅客户端无法加载对应规则集。

已在临时根目录复现两种产物，并用 sing-box 1.14.1 编译合法 SRS：sing-box 加载成功；同一文件作为 Mihomo MRS provider 启动时明确报 `invalid input: magic number mismatch`。Mihomo `-t` 仍退出 0，因此现有静态配置门不能证明实际规则提供器工作。

建议：为不同客户端分别存储 URL 和规则集类型/behavior；仅向匹配格式的客户端发布，或要求成对提供对应格式。回归必须实际加载二进制规则集，不能只匹配 URL 或 RULE-SET 字符串。

## Comments

2026-10-20：已修复。保留现有存储结构，按扩展名仅向对应客户端生成声明和引用；两端需要同一策略时分别注册两个名称/URL。四模板、standard/minimal、全部版本工件的隔离测试通过；新增真实 Mihomo 运行测试，实际编译 MRS 并从 API 确认规则数量大于零。minimal 与 Mihomo 1.18 仍不加载远程集。
