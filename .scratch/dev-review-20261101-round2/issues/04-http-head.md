# 订阅 HEAD 与 ACME 方法边界

Type: task
Status: resolved

订阅 HEAD 复用 GET 的鉴权、路由、query 和只读流量逻辑，保持表示长度与响应头，线上不发送正文；ACME 只接受 GET/HEAD，其他方法返回 405 并提供 Allow。保留 no-store 与既有连接隔离。

Windows 35 项 serve 测试及 18 项订阅 CLI 定向测试通过；Linux 全量及真实 listener/TLS 回归通过。VPS 最终普通构建通过全部 13 格式、13 QR 与 index 的 GET/HEAD 比较，核对状态、头、正文长度和 HEAD wire 无 body。无效认证/query/不兼容格式、503、限流、账期退化和 ACME 方法边界由相应回归覆盖；前后账期与工件只读。
