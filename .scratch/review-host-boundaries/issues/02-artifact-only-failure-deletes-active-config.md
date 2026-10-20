# 仅工件更新失败可能删除活动配置
Status: claimed

regenerate(update_active_config=false) 不读取活动配置快照，但写入失败时把 None 传给 restore_replaced，后者将其解释为活动配置原本不存在并删除该文件。需区分“未参与事务”和“参与事务但此前不存在”，并用真实写入错误复现。
