# sbctl v0.0.15

本版本修复观测接口和配置覆写中的六项问题，不改变持久配置格式。

- 观测接口启停持操作锁直到服务激活或恢复完成，避免失败恢复覆盖其他终端已轮换或撤销的订阅凭据。
- Direct HTTPS 的观测接口操作保留 `root:sbctl-cert 0640` 证书权限，恢复路径也重新准备正确的存储权限。
- 观测接口正确读取 `/memory` 的 HTTP chunked 数据流，取得首个 JSON 快照即返回，并使用总读取期限，避免启用接口后命令一直等待、长时间持锁。
- JSON/YAML 覆写按基础文件、分层文件顺序逐个应用，正确执行每层的 `rules_mode: prepend|append|replace`。
- 覆写使用私有临时文件编辑，提交时检查源文件冲突；失败的编辑不会暴露给其他生成命令。
- 清除服务端覆写会重启并检查服务，失败时恢复源文件与原工件；纯客户端清除继续只重新生成。

发布工作流默认生成草稿并上传全部签名资产。VPS 验收通过后发布同一批产物，避免测试构建与发布构建不同。

还包含上一轮已完成但尚未发布的修复：过期配置向导不能恢复旧订阅凭据；SRS/MRS 远程规则集按客户端隔离；fake-ip 精确域名和关键词例外不再遗漏；非法或溢出的规则集刷新间隔在写入前拒绝。

验证：fmt、Clippy、481 个常规测试、真实内核矩阵和三发行版 systemd 验收通过；Actions 生产构建在 VPS 完成真实 API 启停、覆写编辑/清除、证书权限、公网 12 个订阅与二维码解码、五协议 HTTPS 204 转发。测试后恢复 VPS 原部署和定时器，保留 root-only 备份。

测试与发布的 amd64 二进制 SHA-256：`48ced52eb14628ead81ceaa6424cc67b440935802b557f9f5f3b7154e449ee7e`。

详见[验收记录](../.scratch/review-2026-10-20-followup/verification.md)及[Release Actions](https://github.com/xiaolingxiaoying/vps-sub-meter/actions/runs/39465390004)。
