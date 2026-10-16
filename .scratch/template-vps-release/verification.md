# 验证记录

- Linux 完整 Rust 测试、Clippy `-D warnings`、5 个 Python 安装器测试通过。
- sing-box 1.14.2 接受完整标准模板；Mihomo 1.19.30 接受三种模板选项、两种规则配置及两个 YAML 版本的全部 12 种组合。
- 新增保真断言：默认 sing-box 的 DNS、入站、路由、日志与来源模板逐项相等；Mihomo 的基础设置和完整规则顺序逐项相等。
- 指定 Ubuntu 测试 VPS：先在服务器本机私有备份旧部署，再在隔离根目录沿用原域名、五协议凭据、端口及流量配置生成 13 个制品；生成器验证和服务器 sing-box 1.14.2 check 均通过。
- 旧 HTTP 测速地址在隔离测试配置中更新为模板指定的 HTTPS gstatic 地址。旧覆写保存在备份中，隔离生成使用未覆写的原始模板。

在线部署与复测：

- 已部署 GitHub Actions 为提交 `eedc35e7f0355e0b594e15520a366a68e9e08d18` 构建的生产 amd64 制品；SHA256 为 `633a578b2c7a0a45028989a25a9361fb4f931dd2c1ac8290b8b7664366470147`。
- 原域名、五协议凭据与端口、订阅凭据、流量额度及刷新时区均保留；只将旧 HTTP 测速地址改为模板中的 HTTPS 地址。
- 旧覆写已备份并停用，在线配置沿用完整模板。服务器私有备份目录为 `/root/sbctl-template-v0011`。
- 通过原域名和受信任 HTTPS 校验：12 个订阅、12 个二维码、总览页、流量响应元数据；错误凭据、无效格式与查询参数均返回 404。
- 从 VPS 外部再次验证 Mihomo、完整 sing-box 订阅及总览页，证书校验与 HTTP 访问均通过。
- 在服务器本机使用隔离客户端，经过原域名分别连接 VLESS、VMess、Hysteria2、TUIC、AnyTLS；部署前后均为 HTTP 204。此项证明实际代理连接，不代表用户本地网络的 TUN/校园网环境验收。
- sbctl 与 sing-box 分别以 `sbctl`、`sing-box` 用户运行；HTTPS socket 与流量定时器正常。
- Actions 多版本内核校验、双架构生产构建及 systemd 安装/回滚验收均已通过；Release 已正式发布。

发布流程：[Release run 38940312704](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/38940312704)。订阅凭据与私人订阅链接不写入公开报告。


最终发布验证：

- [sbctl v0.0.11](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.11) 为非草稿、非预发布版本，包含双架构 sbctl/sing-box、两个签名 manifest、安装脚本和 SHA256SUMS 共 8 个资产。
- 发布 CI 中 274 个库测试、146 个 CLI 测试通过；其余合并库、发布信任和版本注册表测试通过，真实内核测试由独立矩阵执行。Clippy、Python 安装器测试与 systemd 门禁均通过。
- 在线 `/usr/local/bin/sbctl release verify` 已验证发布 manifest 的 Ed25519 签名。
- 在线 sbctl 与 sing-box 的 SHA256 均与已发布签名 manifest 一致。
- 正式部署后四项服务/socket/timer 均为 active，原有订阅地址继续有效。
