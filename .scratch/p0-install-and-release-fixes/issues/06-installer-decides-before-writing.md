# 06 安装器先决策、后落盘

Status: ready-for-agent

## 问题

`scripts/install.sh` 先把 sbctl 装到 `/usr/local/bin`（`mv -f` + `ln -sf ly`），之后才用
已安装的二进制做只读预检并询问"保留 / 备份后全新安装"。用户选"保留并退出"时磁盘上
已经是新二进制，而配置、工件与 systemd 单元仍是旧版；`sbctl.service` 是
`Restart=on-failure`，之后任一重启都会用新二进制读旧配置。

## 要做

把顺序改成：① 下载并验签 manifest → ② 下载/校验候选 sbctl 到工作目录（不写主机）→
③ 用**候选**二进制做只读预检 → ④ 若发现已有部署，先问"保留并退出 / 备份后全新安装" →
⑤ 确认后才落盘并 `test -x` 复验 → ⑥ 执行对应安装路径。

- "保留并退出"路径打印升级指引（`sbctl update`、`ly`）并保证主机二进制一字节未变。
- 非 TTY 且无 `/dev/tty` 时行为不变（退出 2），但同样不替换二进制。
- 交互输入解析统一成 `resolve_installer_input`，`--guided` 路径也要先解析。

## 验收

- `tests/acceptance/verify-bootstrap.sh`：
  - 场景 1（不变）：安装器把操作员参数原样转发给 sbctl；
  - 场景 2（新增）：用一个"预检报告已有部署"的候选 stub + 已签 manifest，先在
    `/usr/local/bin/sbctl` 放一个已知文件并记录 sha256；无 TTY 运行必须失败且哈希不变；
    再用 `script -qec` 提供 pty 选"1"，输出必须包含 `sbctl update` 且哈希仍不变。

## 实现

`scripts/install.sh`、`tests/acceptance/verify-bootstrap.sh`。
