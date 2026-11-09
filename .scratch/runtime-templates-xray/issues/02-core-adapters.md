# 02 抽离 Rust 内核适配并分离客户端校验

Status: ready-for-agent
Type: task
Blocked by: 01

## 工作

增加 CoreKind、core 缺省反序列化和小范围内核操作接口，将 sing-box 现有行为迁入适配器。重点改造 config、subscription/artifacts、lifecycle、observe、update 的内核参数边界，保留旧 CLI 参数和行为。

将服务端配置生成/校验与客户端产物生成/校验分开，落实 01 确定的 full-profile 默认策略；Xray 模式不能读取 Xray 版本去推断 sing-box 版本，也不能调用 Xray 检查客户端 JSON。旧 sing-box 默认部署不得因抽象改造意外变化。

## 验收

- 无 core 字段的现有配置加载、序列化迁移和既有 sing-box 测试通过。
- 默认生成快照保持不变；有意变化单独说明。
- 测试证明服务端适配器和客户端校验器相互独立，无客户端真核时明确报告检查范围。
- 接口覆盖身份/版本、能力、生成、检查、启动描述；不引入脚本执行或动态插件框架。
