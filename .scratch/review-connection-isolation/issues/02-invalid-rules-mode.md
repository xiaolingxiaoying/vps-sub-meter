# rules_mode 类型错误被静默解释为 prepend
Status: claimed

JSON as_str().unwrap_or_default() 和 YAML 非字符串分支把布尔、数字、null、数组等解释为空字符串，再作为 prepend 接受。需严格校验显式值并保持失败前配置与工件不变。
