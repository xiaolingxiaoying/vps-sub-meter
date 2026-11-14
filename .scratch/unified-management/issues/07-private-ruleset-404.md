# 07 修复 sing-box 启动时私有 IP 规则集 404

Status: ready-for-human

用户报告 sing-box 1.14.4 在初始化 `geoip-private` 时收到 404。VPS 实测确认内置标准模板的 SagerNet `geoip-private.srs` 地址不存在，另外三个规则集下载正常。此前的配置检查没有初始化远程规则集，未覆盖这一错误。

标准/精简完整模板使用内置 `ip_is_private` 保留私有 IP 直连，移除该远程依赖；统一模板目录中该条目仅保留 Mihomo 表示。更新各版本快照，新增所有模板与客户端版本的回归断言。CI 增加官方 1.14.4 和无缓存真实路由初始化测试，不创建宿主 TUN 或监听端口。

## Comments

2026-11-14：Windows 320 个库测试、8 个统一管理测试及 Clippy 通过；官方 1.14.4 无缓存初始化远程规则集通过。VPS 已部署修正版本并重新发布订阅，三个保留 SRS 均返回 200，代理 PID 保持。结果见 ../private-rule-fix-verification.json。
