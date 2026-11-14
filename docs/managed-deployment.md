# 统一节点与双内核管理

开发分支新增统一管理流程。未迁移部署保留旧行为；迁移保留原节点参数、订阅域名、凭据与已有格式地址。

新机器先使用现有 `sbctl install` 完成基础部署，再执行下列迁移、安装目标内核和模式切换命令。当前新装入口仍从 sing-box 起步。

## 开始使用

```sh
sbctl node migrate                 # 只预览，不修改部署
sbctl node migrate --apply         # 导入现有五协议和伴随 Xray 节点
sbctl node list --json
sbctl core status
sbctl core capabilities
```

修改节点和分组默认只显示计划；添加 `--apply` 才提交。菜单先编辑草稿，再显示差异并确认；取消或 EOF 不保存。节点 ID 与客户端 tag 稳定，修改显示名称不会改变客户端引用。

```sh
sbctl node add --name '备用 WS' --preset vless-ws-tls --apply
sbctl node edit NODE_ID --name '常用节点' --port 23456 --apply
sbctl node copy NODE_ID --name '独立备用' --apply
sbctl node disable NODE_ID --apply
sbctl node move NODE_ID --core xray --apply
sbctl node list --core xray --page 1 --per-page 20
sbctl node check
sbctl node probe NODE_ID
sbctl node export NODE_ID --qr     # 明确显示含节点凭据的分享链接
```

预设包括 `vless-reality`、`vmess-websocket`、`hysteria2`、`tuic`、`anytls`、`vless-xhttp-reality`、`vless-xhttp-tls`、`vless-ws-tls` 和 `trojan-tcp-tls`。新增节点优先选择当前模式下已验证的内核；只受另一内核支持时先切换模式。协议/端口/凭据不再通过旧五协议向导修改。

`node probe` 启动临时回环 SOCKS 客户端，实际通过指定节点请求已配置测速 URL，结束后回收进程和私有临时文件，不重启服务。可用 `--url` 更换 HTTP(S) 测试目标；`--client-bin` 指定对应客户端检查程序。`node check` 只证明配置有效，不能替代转发验证。

## 内核与恢复

```sh
sbctl core install xray            # 固定官方 Xray 26.10.31，验证归档摘要
sbctl core install sing-box        # 固定官方 sing-box 1.14.2，验证官方摘要
sbctl core mode dual --apply
sbctl core mode xray --disable NODE_ID_1,NODE_ID_2 --apply
sbctl core mode sing-box --apply
sbctl core restart xray
sbctl core recover
sbctl core rollback
```

服务端安装需要 Linux；Xray 归档提取需要系统 `unzip`（Debian/Ubuntu：`apt-get install unzip`）。内核版本须在项目验证列表内，未知版本不会自动开放能力。安装二进制不等于启用其服务。管理程序更新不隐式更新两种内核；各内核单独更新和重启。

单内核模式会迁移共同支持的节点；不兼容节点默认阻断，只有显式 `--disable` 后才停用并保留设置。已启用 Hy2 端口跳跃时，先在跳跃配置中关闭，再停用或更改其基础监听节点。相同端口交接会中断现有连接。

应用前检查配置；失败恢复旧配置和受影响服务。中断后使用 `core recover`，开机恢复服务也会检查未完成事务。`core rollback` 只恢复最近、尚未被后续节点修改替代的恢复点；不会恢复已吊销订阅凭据或旧流量账期。

## 分组与订阅

```sh
sbctl group create --name '常用' --members NODE_ID_1,NODE_ID_2 --strategy urltest --apply
sbctl group list
sbctl group edit GROUP_ID --name '日常使用' --apply
sbctl group links GROUP_ID --format sing-box-full --credential-name phone --qr
sbctl sub
```

节点组控制订阅包含的节点及顺序，客户端策略支持 `select` 与 `urltest`。URI 格式只有节点列表。全量链接继续保留；组链接使用 `/sub/<credential>/groups/<group-id>/<format>`，二维码在该组的 `qr/<format>`。组改名不改变链接。组不是设备权限隔离；有有效凭据的管理员可获取其他组。

菜单提供“客户端 → 版本 → 全量/节点组 → 设备凭据”的选择。CLI、菜单与订阅网页共用兼容目录，列出包含/排除数量和原因。无兼容节点的格式不发布空文件；删除的组不回退为全量。XHTTP 与当前 Xray Reality 按目标客户端能力过滤。

在 Xray 26.10.31 上，普通 Reality 节点的 sing-box/Mihomo/Shadowrocket 表示尚未通过互通验证，暂时过滤；URI 保留供兼容 Xray 客户端使用。VMess 分配给 Xray 时显式导出 `aes-128-gcm`，避免 sing-box 1.14 在 TLS 下把 `auto` 选择为 `zero`。自签名 TLS 组合的 URI 补充相应证书校验选项；地址、端口与凭据保持。上述边界来自实际连接测试，不能用“配置能加载”代替。

## 客户端模板

迁移时固定 `sing-box-full.json` 的实际版本档，之后独立选择：

```sh
sbctl core apply --client-version 1.14 --apply
sbctl core apply --apply           # 应用当前节点、分组、模板与覆写
```

可选基础设置位于 `/etc/sbctl/templates/client/sing-box.json` 和 `mihomo.yaml`，支持结构化 DNS、TUN、路由和基础设置。不得包含 `outbounds`、`proxies`、`proxy-groups`；这些由节点和分组管理。未提供文件使用内置默认；文件存在但非法则拒绝发布，保留当前批次。版本字段必须与目标客户端一致。

已有管理员覆写与规则继续最后应用，并检查最终引用。裸 sing-box 节点片段与 URI 不应用客户端模板覆写。只改分组、显示名或客户端设置时，代理配置不变就不重启数据平面。旧 `custom-*` 监听器仍属于高级覆写，不自动纳入订阅。

服务端覆写不得修改托管节点的监听、端口、协议、传输或 TLS 身份；这些由 `node` 命令修改。切换 Xray 单内核前必须先处理 sing-box 服务端覆写，避免静默忽略原有配置。

## 验证入口

Windows：`cargo test --workspace --features sbctl/test-signing`；静态检查：`cargo clippy --workspace --all-targets --all-features -- -D warnings`。

WSL：`scripts/dev/verify-managed.sh` 在独立 ext4 工作副本中运行回归、固定内核检查、代表性转发和新旧菜单伪终端验收；`build-managed.sh` 构建 Linux 候选，VPS 不编译。systemd 发行版门禁与测试 VPS 验收证据另行记录，不能由模拟测试替代。

本轮结果见 [2026-11-14 验收记录](managed-verification-2026-11-14.md)。
