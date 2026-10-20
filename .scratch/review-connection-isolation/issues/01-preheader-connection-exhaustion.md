# 单来源未完成请求耗尽全局连接槽位
Status: resolved

HTTP、ACME 和 TLS 接收循环仅有全局 32 槽位限制。请求级 IpBudget 在完整 HTTP 请求头之后才执行，同一来源能够用未完成的请求头或 TLS ClientHello 占用所有连接。需在接受连接时限制来源并用 RAII 回收。

## 结果

`bd56770` 增加共享连接额度实现，在 HTTP、TLS 和 ACME 接受时限制每来源 8 个活动连接。VPS 上公开 v0.0.15 与 Actions 候选的对照实验确认修复；另覆盖取消回收、全局额度、IPv4 映射和反向代理例外。见 [验收记录](../verification.md)。
