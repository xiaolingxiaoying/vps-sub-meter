# 01 固定版本能力验证与迁移契约

Status: ready-for-agent
Type: research
Blocked by: 无

## 工作

以 [spec](../spec.md) 为目标，选定 sing-box 与官方 Xray 的固定基线版本，核实 amd64/arm64 发布工件、版本输出、检查命令、资产摘要与协议支持。首版必做 VLESS Reality 和 VMess WS；Hy2 文档/构建入口差异必须记录，TUIC/AnyTLS 默认不开放。

设计旧配置 core 缺省兼容、full-profile 版本策略、服务账户/unit 路径、外部模板 schema/优先级/可调字段与受保护字段，写成明确 ADR。明确服务端规则与客户端规则不是同一配置；切换不自动转换不兼容覆写。

## 验收

- 有固定版本、工件来源/摘要、架构和最小配置检查证据，不用 latest 作为持久化版本。
- 两个必做协议至少完成 Linux amd64 客户端到服务端互通原型；arm64 运行作为 08 发布门禁。
- 能力矩阵区分已验证、未验证、不支持；Hy2 没有验证通过不阻塞两协议首版。
- 新 ADR 明确修订 spec 列出的既有边界，确定后更新任务契约；不修改生产部署。

## Comments

2026-11-09：已开始实施，官方 amd64 工件摘要和版本执行通过，新增 Xray 纯配置生成及真核检查用例。进度与尚未通过的验收见 [progress](../progress.md)；本任务尚未完成。
