# sing-box-yg 固定快照

- 上游仓库：<https://github.com/yonggekkk/sing-box-yg>
- 固定 commit：`8faa21d368bc93c0a99b358168fc913d5ab2b9c2`
- 快照日期：2026-07-08
- 许可证：GPL-3.0（完整文本见 `LICENSE`）

本目录只保留一体化脚本所需的上游安装源码、版本信息、说明和许可证。
`vps-sub-meter.sh` 在 VPS 上只会下载并执行上述 commit 对应的 `sb.sh`，不会跟随
上游 `main` 自动更新。升级快照时必须同时更新本文件、`vps-sub-meter.sh` 中的
`UPSTREAM_SINGBOX_YG_COMMIT`，并重新验证节点导出文件：

- `/etc/s-box/clmi.yaml`
- `/etc/s-box/sbox.json`
- `/etc/s-box/jhsub.txt`
