# 回滚和过期工件清理使用固定目录删除
Status: resolved

订阅工件删除及回滚中使用 fs::remove_file 路径，若父目录变成链接可能删到外部同名文件。应复用无链接父目录解析并使用 unlinkat，同时保留已有“缺失时忽略”的行为。

## Answer

订阅工件和活动配置的回滚/过期清理改用无链接父目录解析及 unlinkat；叶子链接只删除链接本身。回归覆盖链接父目录、叶子链接、原父目录被移走后删除仍落在固定目录，以及 stale_artifact_cleanup_cannot_delete_through_a_replaced_parent_link。缺失处理维持原有调用方策略。参见 [验收记录](../verification.md)。
