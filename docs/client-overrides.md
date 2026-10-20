# 不重编译的客户端配置覆写

修改 DNS、TUN、嗅探和分流时，使用 VPS 上的 JSON/YAML 覆写文件。sbctl 每次重新生成订阅时读取这些文件，无需安装 Rust、重新编译程序或修改每个客户端。内置基础模板仍跟随程序版本发布；协议、凭据、节点和各版本适配由程序生成。

## 文件与现成示例

| 目标 | VPS 基础文件 | 分层目录 | 仓库示例 |
| --- | --- | --- | --- |
| sing-box 完整客户端订阅 | `/etc/sbctl/overrides/sing-box-override.json` | `/etc/sbctl/overrides/sing-box.d/*.json` | [sing-box-override.json](../examples/overrides/sing-box-override.json) |
| Clash/Mihomo 客户端订阅 | `/etc/sbctl/overrides/clash-override.yaml` | `/etc/sbctl/overrides/clash.d/*.yaml` | [clash-override.yaml](../examples/overrides/clash-override.yaml) |

示例明确让 `services.googleapis.cn`、OpenAI 和 X/Twitter 域名走 `代理` 组，保留生成的真实节点和凭据。Mihomo 示例还展示境内外 DoH、严格 TUN 路由和取消网段排除；sing-box 示例使用各版本都支持的 DNS `strategy`，不覆盖有版本差异的 DNS 服务器数组。两份示例均纳入真实内核兼容性测试。

这些是客户端订阅设置，不是 VPS 代理服务的入站设置。需要改 VPS 本机服务时使用独立的 `server` 覆写目标。

## 推荐：通过编辑命令应用

在 VPS 上运行，粘贴相应示例内容后保存退出：

```bash
sudo sbctl config override show
sudo sbctl config override edit clash --layer 20-local.yaml
sudo sbctl config override edit sing-box --layer 20-local.json
```

两个编辑命令分别操作两个文件，按需执行。基础文件先合并，分层文件按文件名字典序合并；新层不会删除现有覆写。编辑命令会校验、生成订阅；在托管部署中复用配置事务，失败恢复编辑前的文件。新版本的基础文件编辑器也直接使用上述示例。

已有 `0.0.11` 支持覆写和分层文件，无需等待新版本即可手动粘贴这些示例。旧版基础文件示例若出现 `节点选择`，改为当前组名 `代理`。更新后的内置基础模板与事务修复则需要安装包含相应提交的新发布版本。

## 直接修改文件后应用

如果直接用编辑器或上传文件修改 VPS 上的覆写，保存本地备份后执行：

```bash
sudo sbctl config override validate
sudo sbctl regenerate
```

`validate` 只检查，不发布订阅；`regenerate` 更新生成产物。通过订阅更新影响客户端，客户端随后需要刷新订阅。`validate` 和生成事务可用已安装的 sing-box 校验合并后的完整配置；指定其他内核可加 `--sing-box-bin /path/to/sing-box`。当前检查不会调用 Mihomo，也不会逐一运行客户端所有旧版本内核；自定义含版本差异的字段时应分别导出并用对应客户端内核检查。

直接改文件后校验失败，已发布产物保持原样；手工写入的源文件需要恢复备份或修正。编辑命令的自动恢复只覆盖该命令所管理的编辑。

如需撤销单个分层文件，通过相同 `edit ... --layer ...` 命令把内容改成 `{}`。清除某个目标全部覆写可用 `sudo sbctl config override clear clash` 或 `sudo sbctl config override clear sing-box`；这会同时清除该目标的基础文件和全部分层文件。

## 合并和版本边界

- 对象递归合并，只写需要改的字段。
- `rules` 默认前插；顶层 `rules_mode` 可选 `prepend`、`append` 或 `replace`。`replace` 同时作用于该文件中所有名为 `rules` 的数组，使用时应提供完整规则。
- sing-box `outbounds` 按 `tag`、Mihomo `proxies` 和 `proxy-groups` 按 `name` 合并，避免重写自动生成的节点。
- 其他数组整体替换，包括 DNS `servers`、`nameserver`、Fake-IP 过滤列表和客户端 `inbounds`。调整时保留仍需要的全部元素。
- 覆写在各版本适配之后合并。同一份 sing-box 覆写会作用于所有完整版本；1.12+ 的 typed DNS `servers` 不能直接放进全版本共用覆写，1.10 也不支持 `action: sniff`。示例只用跨版本字段；需要这些设置时优先使用程序已有的 DNS 模式、规则列表和模板配置选项。
- 不要把完整参考模板连同占位 `proxies`/`outbounds` 当作覆写直接粘贴，否则会追加示例节点或替换生成内容。参考模板中需要调整的基础设置可拆成覆写片段。
- 裸节点 `sing-box.json` 和 URI/Shadowrocket 格式不应用客户端覆写。

## 本地例外与远程规则集

fake-ip 例外支持精确域名、域名后缀和关键词；不接受 IP-CIDR：

```bash
sudo sbctl rule add fakeip-filter DOMAIN,music.example DOMAIN-SUFFIX,example.net DOMAIN-KEYWORD,stream
```

远程二进制规则集按客户端格式分别注册。`.srs` 仅写入 sing-box 完整订阅，`.mrs` 仅写入现行 Mihomo 订阅，两种二进制不能互换。需要两端覆盖同一策略时，使用两个名称及对应的两个 URL。minimal 档与 Mihomo 1.18 兼容订阅不加载远程规则集。

```bash
sudo sbctl rule-set add game-sing --url https://example.com/game.srs --outbound proxy
sudo sbctl rule-set add game-mihomo --url https://example.com/game.mrs --outbound proxy
sudo sbctl rule-set interval 12h
```

刷新间隔支持正整数加 `s`、`m`、`h`、`d`，必须在内核可表示的时长范围内；零、溢出和非法输入会在写入前被拒绝。

## VPS 更新和源码推送的区别

`git push` 只更新源码。`sbctl update` 从 GitHub Release 的签名清单下载发布工件，不能直接安装 dev 分支提交；Release 发布完成后才有对应的二进制可更新。

```bash
sbctl --version
sudo sbctl update --check
# 确认签名清单中的版本为目标发布版本后：
sudo sbctl update
sudo sbctl config override validate
```

以上更新命令会同时更新签名清单中的 sbctl 和 sing-box，并执行配置与服务健康检查；失败时回滚。无需在 VPS 编译 Rust，也不要用安装向导重建已有部署来代替更新。
