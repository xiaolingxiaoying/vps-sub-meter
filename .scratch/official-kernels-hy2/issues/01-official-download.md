# 官方唯一下载源
Status: resolved

移除 repository 选项，所有下载从官方归档提取，保留签名二进制摘要校验，Release 不再上传内核副本。覆盖非官方 manifest URL 不被访问的安装回归。

## Comments

已实现并通过 WSL 全量测试、数字交互检查、真实内核订阅矩阵和隔离 Hy2 入站跳跃验收；systemd 生命周期额外加入 CI 三发行版门禁。
