# P3：规则集刷新间隔解析发生整数溢出

Status: resolved
Priority: P3

位置：`src/subscription/render/clash.rs:243`。

extra_rule_set_seconds 把可解析为 u64 的输入直接乘以 60、3600 或 86400，没有 checked_mul 或范围限制。`rule-set interval 18446744073709551615d` 在 debug 构建中触发乘法溢出 panic，退出 101，绕过正常的参数错误返回。未启用 overflow-checks 的 release 构建存在回绕风险（本轮实际执行的是 debug 复现）。该问题需要异常大的管理员输入，优先级较低。

建议：使用 checked_mul 并验证大于零、落在下游内核支持的时长范围内，超出时返回 None；补充单位乘法的边界测试。

## Comments

2026-10-20：已修复。使用 checked_mul，限制为正数及 Go Duration 的可表示范围；同时避免 UTF-8 单位输入触发 split_at panic。持久配置校验复用同一解析器；CLI 边界测试证明零、超大值和 Unicode 非法值均退出 2，且配置、工件不变。
