# P2：sing-box 静默丢弃 fake-ip 精确域名和关键词例外

Status: resolved
Priority: P2

位置：`src/rule_list.rs:257`。

fakeip-filter 与其他规则列表共用支持 DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、IP-CIDR 的校验器，但 extend_fake_ip_dns_rule 遇到非 DOMAIN-SUFFIX 就直接 continue。管理员添加精确域名或关键词后 CLI 返回成功，源列表保留记录，Mihomo 也有对应例外，sing-box DNS 配置却完全没有记录，因此仍可能返回 fake IP。

复现：`rule add fakeip-filter DOMAIN,exact-review.example DOMAIN-KEYWORD,keyword-review DOMAIN-SUFFIX,suffix-review.example --sing-box-bin <真实内核>` 返回 0；sing-box-full 的 dns 仅出现 suffix-review.example，Mihomo 出现全部三项。实际 sing-box 1.14.1 检查通过，因为遗漏规则不会引起语法错误。

建议：分别渲染精确域名、后缀和关键词匹配，保持例外规则的 OR 语义；无法支持的类型应在写入前明确拒绝。补充所有受支持匹配器跨客户端的产物断言。

## Comments

2026-10-20：已修复。后缀继续原位扩展；精确域名和关键词各自生成独立 DNS 规则，放在 fake-IP 通配规则之前。fake-ip 列表的 IP-CIDR 在 CLI 和手工源文件加载时明确拒绝。新增产物和 CLI 回归，并通过 sing-box 五版本 × 四模板的 20 次真实配置检查。
