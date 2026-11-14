# 统一节点、双内核管理与订阅体验

Created: 2026-11-14
Status: in-progress

本规格落实用户批准的 dev 分支完善计划。保留 Rust、数字菜单、单管理员单 VPS；不增加 Web 管理面板、外部订阅聚合或链式代理。用户另授权在指定测试 VPS 覆盖配置并验收；登录凭据不得进入仓库、argv 或报告。

## 决策

- `/etc/sbctl/nodes.json` 是迁移后的节点与组权威源；旧配置五协议和 protocol-presets.json 只导入一次，旧命令禁止再改旧协议源。
- 节点 ID、客户端 tag、显示名称分离；改名保持引用；复制生成独立凭据和端口。
- 模式 sing-box / xray / dual；一个节点只归属一个内核。共同组合可迁移，不兼容节点须显式停用；已有 Hy2 跳跃须先显式关闭再停用基础节点。
- 客户端版本在迁移时固定，之后不跟随服务器版本。运行时客户端模板是可选的结构化基础设置，不得替代权威节点或策略组。
- 节点组按 ID 排序引用节点，select/urltest 为首版策略；组 ID 形成稳定路径，改名不更换链接。组不是权限边界。
- 菜单草稿和 CLI 预览使用同一计划与事务。客户端输出完整批次写入后，通过单一指针发布；HTTP 不读取 root 专用源或私钥。
- 事务日志用于恢复中断，最近恢复点用于回退；历史回退不恢复订阅凭据或流量状态。数据平面和订阅 HTTP 生命周期独立。
- 官方内核下载固定版本与摘要校验；未知版本不自动开放能力。Xray 安装依赖系统 unzip，无额外编译工具链。

## 任务

1. [节点模型与迁移](issues/01-model-and-migration.md)
2. [双内核生命周期与恢复](issues/02-runtime-and-recovery.md)
3. [节点管理与节点组](issues/03-node-and-group-management.md)
4. [订阅批次与模板](issues/04-subscriptions-and-templates.md)
5. [数字菜单与兼容入口](issues/05-menu-and-compatibility.md)
6. [验收、CI 与文档](issues/06-validation-and-delivery.md)

## 验收

旧地址与未修改节点参数保留；三种模式互转；多实例、组顺序、兼容过滤正确；取消与 EOF 不提交；非法模板、坏配置、启动失败及中断可恢复；根目录隔离测试不操作宿主 systemd；真实 VPS 验证非 root 服务、证书访问、节点转发与订阅 GET/HEAD/QR。

## 本轮交付

P0–P4 的代码和可执行测试已落地。真实 VPS 完成三种模式、9 节点转发、UDP、订阅凭据、模板和真实重启恢复。P5 的发行版完整 systemd 矩阵、GUI 导入和签名发布尚待执行；规格保持 in-progress。详细边界、使用方式与结果见 docs/managed-deployment.md、docs/managed-verification-2026-11-14.md 及 verification.json。旧单活动内核约束由 ADR-0041 取代。
