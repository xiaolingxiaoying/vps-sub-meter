# 安装、协议和旧版升级

两台 VPS 的核查结果是旧部署 5 个节点、新部署 9 个节点，原因是启用的协议不同。
更新管理程序不会自动补齐协议。节点组只显示组内成员；客户端不支持的协议也会被过滤。
菜单首页和订阅页现在显示各格式的实际数量。完整记录见
[两台 VPS 核查](two-vps-verification-2026-11-16.md)。

## 首次安装

从官方 Release 下载安装器，运行后选择「首次安装」。默认使用官方 sing-box、
常用 5 协议；选择全部 9 协议会另装官方 Xray，增加一个常驻进程。
有域名时填订阅域名并按提示配置 DNS；没有域名时可接受自动探测的公网出口 IP。
IP 模式默认使用 2080 提供 HTTP 订阅，链接含私密凭据，不能当作公开链接分享。
NAT 主机只把探测的 IP 用于对外地址，监听所有本地接口；仍需宿主机端口映射和防火墙放行。

Reality 的 SNI 默认 `www.cloudflare.com`。IP 模式的证书协议默认 `www.bing.com`
与自签证书配合；域名模式按实际证书填写。默认域名在具体网络不可达时可以修改。
SNI 是 TLS 握手中的服务器名称，不是订阅地址，也不要求等于节点出口 IP。

## 旧版升级与统一协议

以下脚本随新 Release 发布。源码中的 `install.sh` 只有公钥占位符，不能直接安装。

```bash
curl -fL -o upgrade.sh https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/upgrade.sh
sudo bash upgrade.sh --check       # 仅检查是否存在旧部署
sudo bash upgrade.sh               # 官方签名校验，新程序执行升级
bash protocols.sh                 # 预览补齐协议和名称
sudo bash protocols.sh --apply     # 需要时安装官方内核，再提交
```

升级保留订阅地址、节点凭据、节点组和流量账期，不依赖旧 CLI 可执行。
补齐操作保留现有节点与端口，增加缺少的协议；已有协议被停用时会重新启用首个匹配节点。
节点组成员保持原样，因此原组不会自动包含新增协议。全部订阅会包含所有兼容节点。
原先自定义名称会统一为协议组合名，同类多个节点追加数字。引用旧节点标签的自定义覆写
需要同步调整；验证失败会拒绝提交。原有额外节点不会删除。

不能承诺修复已丢失的凭据、任意手工破坏的配置或不属于 sbctl 的服务。
这些情况会停止并说明原因，保留原文件。管理程序升级与内核升级分开执行，
失败可用 `sbctl core rollback` 恢复最近事务；恢复前可先查看 CLI 帮助和状态。

## 节点、订阅与网站共用 443

`sbctl port status` 显示节点对外端口、实际监听和 TCP/UDP 归属。
订阅的端口与节点端口独立。443 上 TCP 和 UDP 可以并存；Hysteria2 与 TUIC 都用 UDP，
二者不能同时占同一地址的 UDP 443。

自动 HTTPS 订阅模式可以按不同 SNI 分流 TCP 443，无需另装 NGINX 或 HAProxy：

```bash
sbctl node list
bash share-443.sh --node <节点ID> --website-sni web.example.com --website-port 8443
sudo bash share-443.sh --node <节点ID> --website-sni web.example.com --website-port 8443 --apply
```

网站先改为在 `127.0.0.1:8443` 提供 HTTPS，域名指向 VPS。订阅使用自己的域名；
节点按其 SNI 匹配，Reality 可保留默认伪装 SNI。各入口的 SNI 必须不同。
转发保留原始 TLS，不解密，也不缓存流量到磁盘。网站后端看到的来源是本机，
此实现不注入 PROXY protocol。网站 HTTP/3 应关闭或使用其他 UDP 端口，以免与 QUIC 节点冲突。
只有 IP、相同 SNI、ECH 隐藏 SNI、多个 UDP 443 服务不能用此方式分流。
移除规则可运行 `sbctl port remove <SNI> --apply`；节点会恢复其后台高端口。

## 自定义下载和邮件

内核菜单默认选择官方来源。自定义来源需要直接下载文件的 HTTPS 链接和可信发布者提供的
SHA-256；支持 Linux ELF、sing-box 标准 tar.gz、Xray zip，不接受网页或安装脚本。
链接可以指向用户自有服务器，但仍须是 CLI 列出的已验证内核版本。

```bash
sbctl core install sing-box --version 1.14.2 --url 'https://example.com/core.tar.gz' --sha256 '<64位哈希>'
```

邮件菜单提供逐项设置、隐藏密码输入和最近成功发送时间。465 默认隐式 TLS，587 默认 STARTTLS；
推荐使用邮箱授权码。日常报告默认不含私密订阅链接，网络发送期间不占用部署锁。
手动测试：`sbctl email send`。SMTP 失败显示原因，不把授权码打印出来。

## 本次验证（2026-11-16）

- Windows / WSL 的格式检查、Clippy 与 Linux 工作区回归通过。
- PTY 覆盖菜单、首次安装默认值、取消、协议草稿与分组；订阅 13 格式与 QR、HEAD、凭据拒绝通过。
- 真实 sing-box 1.10–1.14、Xray 26.10.31、Mihomo 的托管配置及兼容客户端验证通过。
- 原生 SNI 分流通过真实 sing-box VMess + WebSocket + TLS 完成 HTTP 转发，同时订阅 HTTPS 正常。
- 实际 sing-box 1.14.1 候选通过自定义下载入口升级旧字段配置，恢复点还原内核与配置通过。
- 旧 CLI 不可执行的管理程序升级、订阅凭据/账期保留、哈希错误、网页伪装下载与 SMTP 锁测试通过。
- Bash 语法、生产签名信任隔离、安装器公钥渲染及工作流 YAML 检查通过。

本次未部署两台 VPS、未发送真实邮件。Docker 引擎未运行，三个发行版的真实 systemd 容器验收
尚需 CI 执行；新增 bootstrap 恢复升级场景已加入该验收脚本。历史服务标记已核对多个已发布 tag，
并不等于每个历史版本都完成实机升级。新脚本需随新版本发布后才能从 Release 下载使用。
