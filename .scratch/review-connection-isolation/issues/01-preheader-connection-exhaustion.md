# 单来源未完成请求耗尽全局连接槽位
Status: claimed

HTTP、ACME 和 TLS 接收循环仅有全局 32 槽位限制。请求级 IpBudget 在完整 HTTP 请求头之后才执行，同一来源能够用未完成的请求头或 TLS ClientHello 占用所有连接。需在接受连接时限制来源并用 RAII 回收。
