# 仅工件更新失败可能删除活动配置
Status: resolved

regenerate(update_active_config=false) 不读取活动配置快照，但写入失败时把 None 传给 restore_replaced，后者将其解释为活动配置原本不存在并删除该文件。需区分“未参与事务”和“参与事务但此前不存在”，并用真实写入错误复现。

## 结果

`d25f562` 明确区分 Untouched 和 Restore，仅尝试写入活动配置后才恢复其快照。VPS 上通过真实内核检查后限制隔离子进程文件大小，确认旧版删除原活动配置，新版保留原内容和 inode，旧工件字节不变。详见 [验收记录](../verification.md)。
