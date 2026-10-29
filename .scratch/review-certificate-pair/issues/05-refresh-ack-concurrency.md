# 继续审查并发发布与刷新确认的代际绑定
Status: resolved
Type: research

v0.0.21 的待刷新标记随目录原子发布，正常顺序的失败重试已有真实核心验证。后续需专门验证两个特权证书命令并发：A 刷新完成后、清除标记前，B 是否可能发布另一代并退出，随后 A 按当前路径清除 B 的标记。此为源码分析的待验证时序，尚未作故障注入，不宣称已稳定复现。

研究方向：确认绑定到调用方已刷新代的目录设备/inode；清除动作通过该代已打开的目录句柄完成，不能检查后再按当前路径删除。验证旧代确认不会删除新代标记，普通重试仍工作。只在临时目录或隔离命名空间实验，不在真实证书制造并发故障。

## Resolution

确认的缺陷：`certificate` 命令在完成 `systemctl try-restart` 后按当前路径删除 `.sbctl-refresh-pending`。若另一特权发布者在“本命令刷新完成、清除标记”之间交换了新的目录代，本命令会删掉新代的标记，使新代的核心刷新确认被误认为已完成。

修复（本次工作树）：

- `clear_refresh_pending` 接收操作后采样的 `PinnedGeneration`（目录 dev/inode），Linux 下打开父目录与目标代目录（逐级 `O_NOFOLLOW`），比较 dev/inode 后仅通过该已打开目录句柄 `unlinkat` 相对删除标记。代不匹配时返回 `false` 并保留新代标记。
- 目标代在确认前被整体删除时返回 `false`，沿用旧实现的幂等语义，不把已完成的操作误报为失败。
- 非 Linux 开发适配仍按路径删除；该代际保证与目录交换发布一样只在 Linux 生产成立（ADR-0037）。
- 回归覆盖：`marker_acknowledgement_cannot_clear_a_concurrent_publishers_generation`（目录层）、`refresh_acknowledgement_is_bound_to_the_published_generation`（证书 API 层）。
- 清除动作现在与 `replace_files_with_marker` 共用证书目录发布锁；并发清除不会再让发布在复制保留项时遇到 `ENOENT`。回归：`acknowledgement_waits_for_an_in_progress_publication`。

残留风险：`(dev, ino)` 在旧代删除后理论上可复用，需两次发布夹一次确认才会触发；已保持记录。
