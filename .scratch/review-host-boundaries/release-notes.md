修复非生产 `--root` 缺少命令替身时回退到宿主机的问题。服务启停、账户管理、证书续期、UFW 和 sysctl 通过统一命令边界执行；隔离目录必须提供自己的命令，只有根目录 `/` 保留真实主机回退。

修复仅重新生成订阅工件时，写入失败的回滚可能删除活动 sing-box 配置的问题。回滚现在区分未参与更新与需要恢复的活动配置，避免读取、删除或重写未触碰的活动文件。

增强偶发观测测试失败时的诊断信息；未把未复现的问题宣称为已修复。

491 项常规 Rust 测试、生产验签边界测试与安装脚本测试通过；GitHub Actions 的三发行版 systemd 和真实内核兼容性门禁通过。Actions 生产产物在授权 VPS 上完成隔离故障注入、真实服务回归、订阅及五协议 HTTPS 转发验证；VPS 已通过签名更新部署到 0.0.17，保留配置、凭据、流量状态和升级前备份。详见[完整验收记录](https://github.com/xiaolingxiaoying/vps-sub-meter/blob/dev/.scratch/review-host-boundaries/verification.md)。
