# 推送与发布
Status: resolved

所有功能和验证完成后提交 dev 并推送；dev CI 全绿后才创建 v0.0.25 Release 构建。签名、systemd 验收全部通过才发布。

## Comments

已保留并推送原 dev 提交，功能提交 e916b7f。dev CI 41394844915 全部通过后才创建标签并触发 Release 41396299306；Release 工作流全部通过，另行核对 Ed25519 生产签名、六个附件及摘要、官方内核 URL 后正式发布。

发布：https://github.com/xiaolingxiaoying/vps-sub-meter/releases/tag/sbctl-v0.0.25

Release 附件为两个 sbctl 架构、两份签名清单、install.sh 和 SHA256SUMS，无内核副本。旧版升级使用安装脚本的兼容说明已写入 Release。
