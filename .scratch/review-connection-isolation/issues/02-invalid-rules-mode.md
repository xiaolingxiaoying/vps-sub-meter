# rules_mode 类型错误被静默解释为 prepend
Status: resolved

JSON as_str().unwrap_or_default() 和 YAML 非字符串分支把布尔、数字、null、数组等解释为空字符串，再作为 prepend 接受。需严格校验显式值并保持失败前配置与工件不变。

## 结果

`bd56770` 严格接受三个字符串值；缺省仍使用 prepend。Rust 与 VPS 上 Actions 生产二进制均确认 JSON/YAML/client/server 错误值会失败，且已保存配置和工件字节未变。见 [验收记录](../verification.md)。
