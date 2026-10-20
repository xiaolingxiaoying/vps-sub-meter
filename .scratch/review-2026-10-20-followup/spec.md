# 代码审查续轮（2026-10-20）

基线：`5eb1cc2`，工作区原本干净。本轮只审查和隔离复现，未修改产品代码、提交或操作 VPS。

## 修复进展

管理员随后授权修复、Actions 构建、指定 VPS 验收及发布新 Release。五项修复已实现，并增加 4 个 CLI/工件级回归与真实 Linux UID/GID 证书权限检查。fmt、所有目标/特性 Clippy、479 个常规测试和 7 个真实内核测试通过。VPS 验收和发布结果随后补充；以下原始复现记录保留以供追溯。

## 已确认问题

1. [P1：观测接口失败恢复重新启用旧订阅凭据](issues/01-api-rollback-restores-credential.md)
2. [P1：观测接口启停破坏 Direct HTTPS 证书权限](issues/02-api-breaks-direct-certificate-access.md)
3. [P2：覆写 rules_mode 未作用于生成配置](issues/03-override-rules-mode-ignored.md)
4. [P2：失败的覆写编辑仍可被并发生成发布](issues/04-override-editor-publishes-uncommitted-source.md)
5. [P2：清除服务端覆写没有激活新配置](issues/05-clear-server-override-skips-restart.md)
6. [P1：VPS 验收发现 memory 流响应使观测命令不结束](issues/06-memory-stream-does-not-complete.md)

## 验证与复现

`scripts/dev/wsl-gate.sh` 设置 `SKIP_CLIPPY=1` 执行成功：fmt 和启用 `test-signing` 的整个 workspace 常规测试均通过。未重跑 Clippy、真实内核完整矩阵或 Docker/systemd/VPS 验收。

```powershell
wsl -d Ubuntu-22.04 -- python3 -u /mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/.scratch/review-2026-10-20-followup/reproduce.py
wsl -d Ubuntu-22.04 -u root -- python3 /mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/.scratch/review-2026-10-20-followup/reproduce_tls_permissions.py
```

脚本默认使用 `/home/ly/ws/target-vps/debug/sbctl`，由本轮 gate 同步当前源码后构建。第一条命令覆盖问题 1、3、4、5，全部断言成功；问题 3 同时验证 JSON/YAML 的 append 和 replace。第二条命令覆盖问题 2，真实 Linux UID/GID 验证通过。

所有部署均为自动清理的 `/tmp/sbctl-review-*` 临时目录。sing-box 配置检查和 systemctl 由本地桩模拟；第二个脚本仅在一次性 chroot 内使用根路径运行产品逻辑，不创建或修改宿主账号，不调用真实 systemd。证书权限检查使用临时占位文件，证明读取权限丢失，未宣称进行了真实 TLS 握手。

复现输出：

```text
CONFIRMED api rollback: successful credential rotation was undone; old secret restored
CONFIRMED rules_mode=append: generated rules retained; override still prepended
CONFIRMED rules_mode=replace: generated rules retained; override still prepended
CONFIRMED override clear server: active file changed but no service reload/restart occurred
CONFIRMED override editor: failed edit restored source, but concurrent rule add published rejected edit
CONFIRMED Direct API certificate access: gid 65002 -> 65001; mode 0640 retained; sbctl with sbctl-cert group lost read permission even after rollback
```
