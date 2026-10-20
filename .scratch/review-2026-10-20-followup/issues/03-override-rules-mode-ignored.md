# P2：覆写 rules_mode 未作用于生成配置

Status: needs-triage

位置：`src/override_template.rs:197–205`、`:234–242`；最终应用位于 `src/subscription/artifacts.rs:1145–1177` 及服务端覆写合并路径。

加载时删除每个文档的 `rules_mode`，仅在覆写层之间合并时使用。第一份文档的模式直接丢失；合并后的文档再与生成配置合并时，客户端和服务端均使用默认 prepend 策略。因此配置 `append` 仍然优先于生成规则，`replace` 也无法移除生成规则，与使用指南和 ADR-0029 不符，可能改变实际分流结果。

## 隔离复现

`../reproduce.py` 的 `rules_mode()`：分别在 sing-box JSON 和 Clash YAML 基础覆写中指定 append/replace，仅放一条合法规则。regenerate 成功；sing-box 完整产物仍保留多条生成规则且自定义规则排首位，Clash 中也保留自定义与生成的多个 MATCH 规则。

现有 `a_rules_mode_of_append_is_consumed_and_not_forwarded` 单测手动传入 Append 策略，未覆盖真实应用路径。

## 修复方向

保留各层策略并按顺序应用于生成配置，避免先压平文档后丢失 append/replace 语义；增加最终工件级回归。
