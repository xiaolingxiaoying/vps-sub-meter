# 验证记录

- Linux 完整 Rust 测试、Clippy `-D warnings`、5 个 Python 安装器测试通过。
- sing-box 1.14.2 接受完整标准模板；Mihomo 1.19.30 接受三种模板选项、两种规则配置及两个 YAML 版本的全部 12 种组合。
- 新增保真断言：默认 sing-box 的 DNS、入站、路由、日志与来源模板逐项相等；Mihomo 的基础设置和完整规则顺序逐项相等。
- 指定 Ubuntu 测试 VPS：先在服务器本机私有备份旧部署，再在隔离根目录沿用原域名、五协议凭据、端口及流量配置生成 13 个制品；生成器验证和服务器 sing-box 1.14.2 check 均通过。
- 旧 HTTP 测速地址在隔离测试配置中更新为模板指定的 HTTPS gstatic 地址。旧覆写保存在备份中，隔离生成使用未覆写的原始模板。

后续：通过 GitHub Actions 发布门禁后部署正式 Release，并验证原域名 HTTPS 订阅与五协议数据流。
