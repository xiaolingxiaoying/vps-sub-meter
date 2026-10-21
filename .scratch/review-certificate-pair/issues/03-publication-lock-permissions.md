# 发布锁不能随 PEM 文件一起授权给服务账号
Status: claimed

目录代际发布新增 certificates/.operation.lock。原自签名授权对目录内所有文件 chmod 0640，会使服务组能够打开并 flock 此发布锁；本地被入侵的服务账号可阻塞特权发布。锁须保持 root 拥有、0600，并在更新恢复中维持此策略；真实 systemd 安装及失败更新后验证两个服务账号均不可读该锁。
