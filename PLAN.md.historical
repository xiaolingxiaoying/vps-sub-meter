### 一、 您的原始需求与问题

* **项目背景**：您正在结合使用 `xiaolingxiaoying/vps-sub-meter`（VPS 流量统计与订阅管理）和 `yonggekkk/sing-box-yg`（节点生成）两个脚本。
* **核心目标**：在原有的 Clash 和 sing-box 订阅之外，新增一个专门供 **Shadowrocket** 软件使用的订阅链接，并且要求该链接**保留原脚本中显示流量剩余与订阅到期时间的功能**。
* **具体疑问**：您注意到 `sing-box-yg` 脚本生成了多个包含协议链接的文本文件，特别询问了 `jh_sub.txt` 与 `jhdy.txt` 这两个聚合文件的区别，并希望基于此给出在 `auto_setup.sh` 中添加 Shadowrocket 订阅的代码方案。

### 二、 文件分析结论

经分析 `sing-box-yg` 生成的目录内容：

* **`jh_sub.txt`**：是聚合的**明文**订阅文件，包含所有节点（VLESS、VMess、Hysteria2 等）的原始 URI 链接（每行一个）。
* **`jhdy.txt`**：是将 `jh_sub.txt` 中的明文内容进行 **Base64 编码**后得到的文件。
* **选型建议**：Shadowrocket 和 V2rayN 等客户端通用的标准订阅格式是 Base64 编码，因此**应使用 `jhdy.txt**` 作为 Shadowrocket 的订阅源。

### 三、 代码修改方案（针对 `auto_setup.sh`）

为了让 Shadowrocket 能获取节点并读取 `subscription-userinfo` 流量头信息，需将 `jhdy.txt` 作为新的 `.txt` 路由暴露出来。共需在 `auto_setup.sh` 中修改 6 处：

1. **初始化订阅文件副本（[4/8] 阶段）**：增加逻辑，将 `/etc/s-box/jhdy.txt`（如果不存在则回退至 `jh_sub.txt`）复制到服务目录下的 `/var/lib/subsrv/client.txt`。
2. **添加定时同步机制（[5/8] 阶段）**：在生成 `refresh_sub_copy.sh` 时，追加同步 `.txt` 文件的逻辑，确保 Shadowrocket 订阅内容随上游每 5 分钟更新。
3. **修改 Python 订阅服务端（[7/8] 阶段）**：在 `sub_server.py` 中增加对 `SUB_TXT_PATH` 的环境变量读取，并在 `ROUTE_MAP` 中注册 `.txt` 路径，将 MIME 类型设为 `text/plain`（触发流量头注入）。
4. **注入 Systemd 环境变量（[7/8] 阶段）**：在 `sub-server.service` 中补充 `SUB_TXT_TOKEN_PATH` 和 `SUB_TXT_PATH` 环境变量。
5. **更新 Caddy 反代路由（[8/8] 阶段）**：修改 `Caddyfile` 生成逻辑，将 `.txt` 路径加入到 `@sub_path`（BasicAuth 鉴权）和 `@sub_with_token`（Token 免密访问）的匹配规则中。
6. **完善终端输出信息（脚本末尾）**：在部署完成的提示信息中，新增打印 Shadowrocket 专属的 `.txt` 订阅链接（推荐使用 Token 免密方式）以及对应的二维码生成代码。