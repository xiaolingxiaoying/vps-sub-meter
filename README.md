先运行yonggekkk/sing-box-yg脚本
```bash
bash <(wget -qO- https://raw.githubusercontent.com/yonggekkk/sing-box-yg/main/sb.sh)
```
再使用
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/auto_setup.sh)
```
切换ipv4或ipv6出站
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/switch_sb_mode.sh)
```
gcp
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/gcp_sub_meter.sh)
```

aws
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/aws-sub-meter.sh)
```

vmiss
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/vmiss_sub_meter.sh)
```

nexus
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/nexus-sub-meter.sh)
```

## sbctl 服务端管理工具（Rust 试用分支）

本分支保留原有 VPS 流量统计脚本，并新增独立的 sbctl 服务端管理工具，位于 sbctl/。它支持 Debian 12 / Ubuntu 22.04+、systemd、amd64 和 arm64。功能、部署边界和完整安装说明见 [sbctl/README.md](sbctl/README.md)。

### 在全新测试 VPS 上构建并安装当前分支

以下命令会在 VPS 上从源码构建 sbctl，安装到 /usr/local/bin，然后启动引导安装。请只在可丢弃的测试机执行：引导安装会下载并安装 sing-box、写入 /etc/sbctl、创建 systemd 服务并占用所选端口。安装器默认不改防火墙。该分支没有正式 Release 签名；测试时不要运行 sbctl update，它指向原项目的正式签名 Release。

以有 sudo 权限的普通用户执行：

~~~bash
sudo apt-get update
sudo apt-get install -y ca-certificates curl git build-essential cmake perl pkg-config

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
rustup toolchain install stable
rustup default stable
rustc --version  # 需要 Rust 1.85 或更新版本

SBCTL_TEST_DIR="$(mktemp -d /tmp/sbctl-branch-test.XXXXXX)"
git clone --depth 1 --branch codex/vps-override-cli-fixes \
  https://github.com/xiaolingxiaoying/vps-sub-meter.git "$SBCTL_TEST_DIR/repo"
cd "$SBCTL_TEST_DIR/repo/sbctl"
cargo build --release --locked -p sbctl --no-default-features
sudo install -o root -g root -m 0755 target/release/sbctl /usr/local/bin/sbctl
sudo /usr/local/bin/sbctl install </dev/null && \
  sudo /usr/local/bin/sbctl install --guided
~~~

该命令面向全新主机。若主机上已有 sing-box 部署，预检会停止安装，请勿在生产机选择接管现有服务。
