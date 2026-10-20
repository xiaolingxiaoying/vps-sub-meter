# 继续代码审查（2026-10-20）

审查基线：`66e2fca`（sbctl 0.0.14）。本轮只审查和复现，未修改产品代码或部署服务。

已确认问题，按优先级排列：

- [P1：配置向导恢复已轮换失效的订阅凭据](issues/01-wizard-restores-rotated-credential.md)
- [P2：远程规则集混用 SRS 与 MRS](issues/02-rule-set-binary-format.md)
- [P2：sing-box 静默丢弃 fake-ip 精确域名和关键词例外](issues/03-fakeip-matchers-dropped.md)
- [P3：规则集刷新间隔解析发生整数溢出](issues/04-interval-overflow.md)

复现入口：`reproduce.py`。从 Windows 执行：

```powershell
wsl -d Ubuntu-22.04 -- python3 /mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/.scratch/review-2026-10-20/reproduce.py
```

脚本使用临时根目录、WSL 构建的 sbctl、已有 sing-box 1.14.1 和 Mihomo。systemctl 由临时脚本模拟，不控制宿主服务；规则集注册的生成检查使用 `/bin/true`，另以真实内核独立核验二进制规则集格式。

验证：`scripts/dev/wsl-gate.sh` 的 fmt、Clippy、现有工作区测试全部通过，退出码 0。新的复现脚本确认四项缺陷。真实内核验证发现 Mihomo `-t` 会通过 SRS-as-MRS 配置，但启动后规则提供器报 `invalid input: magic number mismatch`；因此仅配置检查不足以覆盖规则集载入。

本轮未执行公网 VPS、真实 systemd 或 Docker 发布验收。

## 修复完成（2026-10-20）

四张问题单均已解决；实现和测试说明见 `docs/project-review-fixes-2026-10-20.md`。`reproduce.py` 已改为断言修复后的行为，并保留 SRS-as-MRS 的真实内核负例。WSL gate：475 个常规测试通过；普通构建签名信任测试 1 个通过；真实内核 gate 的 7 个测试全部通过，包括新增 fake-ip 兼容性矩阵和 MRS 运行时加载测试。

## 提交与 VPS 核验

按管理员授权提交 `8f614c6`，再以该提交的带生产验签公钥 release 构建验证指定 VPS。四项隔离回归、公网 12 个订阅与二维码、五协议真实 HTTPS 转发均通过。测试后恢复原线上程序、工件及权限，恢复流量定时器；保留 root-only 备份，移除临时候选及回滚定时器。详细结果见上述修复文档的“授权 VPS 验证”一节；没有发布 Release 或永久升级 VPS。
