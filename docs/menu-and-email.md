# 菜单、配置导出与邮件

运行 `sudo ly`（或 `sudo sbctl menu`）。安装与部署先选择内核来源：

- 本仓库 Release：签名 manifest 固定的原版二进制，版本不可选。
- 官方仓库：输入 `1.14.1` 等版本号，或 `latest` 选择最新稳定版。

已有部署切换内核：`sbctl sing-box fetch --source official --version 1.14.1`；
本仓库版本：`sbctl sing-box fetch --source repository`。旧版本可能不支持已启用协议，
实际配置检查失败会保留旧内核；官方归档缺失发布摘要时拒绝安装。

「配置与路由规则」可编辑服务端或客户端 JSON / Clash YAML、预览整体配置并导出。
服务端自定义入站必须使用 `custom-` 前缀 tag；入站、出站按 tag 合并，路由规则默认前插。
管理节点的认证字段仍不能覆写。自定义入站不会自动生成订阅节点。
服务端覆写例如：

```json
{
  "inbounds": [{"type": "mixed", "tag": "custom-local", "listen": "127.0.0.1", "listen_port": 1080}],
  "outbounds": [{"type": "direct", "tag": "custom-direct"}],
  "route": {"rules": [{"inbound": ["custom-local"], "outbound": "custom-direct"}]}
}
```

```bash
sbctl config preview --format server
sbctl config export --format server --output /root/sing-box-server.json
sbctl config export --format clash --output /root/clash.yaml
# 在自己的电脑执行：
scp root@VPS:/root/sing-box-server.json ./sing-box-server.json
```

导出包含凭据，Linux 文件权限为 0600，已有文件不会被覆盖。

「服务与诊断」可以查看日志、完整内核状态、当前连接、证书状态、签发和续期。
连接列表需显式启用观察 API，仅监听 127.0.0.1，并使用随机凭据。
「流量与账期」按所选网卡统计本周期 RX / TX；统计范围是整个 VPS 网卡，包含非代理流量。

新部署的 Clash 各组统一使用 `https://www.gstatic.com/generate_204`，已有部署使用其
`client_latency_probe_url`。该字段也控制 sing-box 客户端测速；需要使用本地可访问的测速站时，
可在完整配置向导 / 客户端模板配置中设置。更新后重新生成工件，再让 Android 客户端刷新订阅。
DIRECT 在无法直连测速站的网络中仍可能超时；节点测试需确保 VPS 能访问测速站。

## 邮件

```bash
sudo sbctl email configure
sudo sbctl email status
sudo sbctl email send
sudo sbctl email enable
# 关闭定时发送：
sudo sbctl email disable
```

配置编辑器优先使用 VISUAL / EDITOR，否则尝试 vim、nano、vi。修改后验证并存为
`/etc/sbctl/email.toml`（root-only 0600）。示例字段：

```toml
smtp_host = "smtp.example.com"
smtp_port = 465
implicit_tls = true
username = "admin@example.com"
password = "SMTP授权码"
from = "admin@example.com"
to = "recipient@example.com"
daily_report = true
include_subscription = false
reset_reminder_hours = 24
```

465 通常使用 `implicit_tls = true`；587 使用 `implicit_tls = false`，强制 STARTTLS，
两者均验证服务器证书。需 curl 的 SMTP 支持和 VPS 可达的 SMTP 出口。
`include_subscription = true` 才会将私有订阅链接放入邮件。邮件包含服务状态、内核版本、
RX/TX、本周期总用量、流量额度以及 VPS 刷新 / 客户端显示时区的下一次刷新时间。
`daily_report` 按客户端显示时区每日最多一次；`reset_reminder_hours = 0` 关闭提前提醒。

定时器每小时检查：日报与每个账期的提前提醒可合并为一封；SMTP 失败不会记录为已发送，
下一轮重试。手动 `email send` 不受去重限制。观察任务：

```bash
systemctl status sbctl-email.timer
journalctl -u sbctl-email.service --no-pager -n 30
```

定时服务以 root 读取私有 SMTP 配置；邮件开关独立于证书 ACME 联系邮箱。

域名证书模式的协议 SNI 必须与订阅域名一致。Direct 首次安装会先启动 ACME 所需的服务，
签发证书后再切换协议监听器到已验证的证书链；签发失败会回滚域名协议安装。
公网 80 端口和域名解析仍需可用，外部反向代理模式的证书由反向代理管理员管理。
