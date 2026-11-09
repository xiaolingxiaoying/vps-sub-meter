# 05 双内核官方下载与签名清单演进

Status: ready-for-agent
Type: task
Blocked by: 01, 02

## 工作

将官方下载扩展到 XTLS/Xray-core，适配 amd64/arm64 资产、压缩包、固定版本输出、摘要校验和受约束解压，只替换预期可执行文件。沿用离线候选工件路径，验证身份后才允许安装。

设计版本化新清单，表达 sing-box 与 Xray 的独立版本/摘要/兼容范围。保留 v1 历史规范化签名验证，不因默认字段加入破坏签名；更新 release.rs、生成/签名脚本、prepare-installer、install.sh 和 release workflow。安装器支持从旧 sing-box 部署升级；更新默认保留当前 core，未知 schema/版本拒绝并清晰诊断。

## 验收

- 两架构的官方工件解析与预期成员定位正确，不通过本仓库镜像下载内核。
- 坏摘要、错误身份、错误架构、非法压缩路径和下载失败不替换活动程序。
- 旧 v1 签名 fixture 与新 schema 均有正反例；生产信任锚和 test-signing 隔离保持。
- sbctl 更新与内核更新分别可用；不会隐式切换、覆盖用户模板或旋转凭据。
