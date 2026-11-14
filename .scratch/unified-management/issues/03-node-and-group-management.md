# 03 节点管理与节点组

Status: ready-for-human

已实现 node/group CLI、节点增改复制启停删除迁移、列表过滤分页、私有类型化输入、分享导出与临时客户端真实连接检查。组按 ID 保存成员与顺序，支持 select/urltest。删除仍被分组引用的节点或破坏最终路由引用的组时拒绝应用。

## Comments

2026-11-14：默认列表与计划脱敏，明确 export/links 才打印凭据。
