此候选未发布，请勿部署。

Actions 39546788910 的构建、基础测试和 systemd 门禁虽成功，VPS 隔离实验仍发现真实 sing-box 1.14.2 在目录交换后继续提供旧证书。后续 v0.0.21 增加受管理数据平面刷新、无变化 pin 保留目录，以及已发布但刷新失败的持久化恢复标记，并把真实核心证书握手检查加入 CI。

此候选测试期间生产 VPS 维持已验收的 v0.0.19，随后完成 [v0.0.21](https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.21) 的全部验收与签名更新。保留此草稿及 tag 作为候选历史，不改写已有 tag。
