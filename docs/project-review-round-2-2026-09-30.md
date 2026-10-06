# sbctl（singbox-sub-me）第二轮项目审查报告

- **审查对象**：`HEAD = 599572c`（2026-09-30 19:09 +0800），工作区另有 27 个未提交文件（+1954 / −829）
- **审查日期**：2026-09-30
- **审查方式**：6 个只读子代理分模块审查（provider `opencode-go`，model `deepseek-v4.1-flash`）+ 主线人工复核，并对最高优先级结论做了**可执行实测**
- **实测环境**：Windows 11；Qt **6.10.3**（`C:\Qt\6.10.3\msvc2022_64`，与 CI 固定版本 `install-qt-action version: 6.10.3` 一致）；rustc/cargo 1.97.1
- **与上一版报告的关系**：上一版 30 条主要结论中 **22 条确认、5 条修正、3 条推翻**，本轮另新增 16 条

> **阅读提示**：本报告对每条结论标注证据等级——
> **【实测】**主线亲自运行命令/程序得到的结果（最强）；
> **【代码确认】**主线亲自打开代码逐行核对；
> **【子代理】**子代理报告且给出了文件行号与机制，主线未逐条复跑；
> **【未复核】**受环境限制（Linux/systemd/远端 GitHub/GUI 运行时）本轮无法验证。

---

## 0. 审查方法与可信度说明

本轮的价值主要来自两件事：

1. **分模块并行审查**：6 个子代理分别覆盖服务端事务核心、安全敏感路径、订阅产物一致性、client-core/json-merge、GUI 四端、测试/文档/CI，共报出 72 条发现。
2. **主线不采信、只采证**：对上一版报告里最“像结论”的几条做了实测或源码级反证，结果推翻了 3 条——其中 2 条是上一版报告的 High 级结论。

同时必须说明两处方法论缺陷，避免读者高估本轮：

- **子代理之间的对抗性复核环节失败**：编排脚本第二阶段（对 72 条发现做独立证伪）返回的 72 条判定全部是 `unverifiable / 复核员未返回该条`，即该环节**没有产出任何有效反证**。本报告的交叉验证由主线人工完成，覆盖对象是 High 级条目与上一版报告的争议条目，而非全部 72 条。
- **子代理的“实测”声明不可全信**：S5 子代理声称“实测确认 sbtui 的 `#[tokio::main]` 默认是 current_thread”，与 tokio 官方实现直接矛盾（见 §2.2）。凡是子代理同时给出“实测”与结论的地方，主线都重新取了证。

---

## 1. 总体结论

| 维度 | 评分 | 说明 |
|---|---|---|
| 服务端事务核心 | 7.0 | 提交点、原子写、flock、健康观察窗设计成熟；回滚权限映射与安装回滚是真实高危 |
| 安全性 | 7.5 | 信任链 fail-closed、常量时间比较、HTTP 边界扎实；供应链摘要缺失不限行是硬伤 |
| client-core | 7.0 | 进程生命周期优秀；系统代理故障路径与持久化原子性分化严重 |
| GUI（GPUI/Slint/QML/TUI） | 6.5 | 单一控制面无违反；**QML 开关绑定系统性失效（本轮实测）**、测试替身掩盖生产缺陷 |
| 测试/文档/CI/发布治理 | 6.0 | 验收套件真实、CI 权限最小化；但发布链无摘要校验、86 个提交从未过 CI、版本号自相矛盾 |
| **综合** | **约 6.8 / 10** | 架构成熟度仍明显高于平均；但“发布治理”和“QML 状态正确性”两块比上一版估计更差 |

**一句话结论**：工程设计的底子很好，本轮没有推翻任何架构级判断；但上一版报告把 GUI 与发布治理估得偏乐观——实测发现 QML 的开关/选中状态在用户第一次交互后就会与后端脱钩，而发布流水线既没有对下载内容做摘要校验、当前 HEAD 也**过不了仓库自己的 `cargo fmt` 闸门**。

---

## 2. 上一版报告中被推翻或需修正的结论（先看这一节）

### 2.1 ~~G-1（QML 本地 JSON 导入完全不可用）~~ → **推翻**【实测】

上一版：`crates/sbgui/qml/pages/SubscriptionsPage.qml:190-204` 里 `new URL(...)` 因 QML 无 WHATWG URL 构造器而抛异常，被 `catch` 吞掉，本地导入功能恒不可用。

实测（Qt 6.10.3，即 CI 固定版本）：

```
$env:QT_QPA_PLATFORM = "offscreen"
qml.exe Probe.qml
qml: typeof URL -> function
qml: constructed ok: file: /C:/tmp/a b.json
qml: localFilePath result -> [C:/tmp/a b.json]
exit=0
```

**Qt 6.10.3 的 QJSEngine 实现了 WHATWG URL**，`new URL("file:///C:/tmp/a%20b.json")` 正常构造，`localFilePath()` 按预期返回 `C:/tmp/a b.json`。上一版报告与 S5 子代理均只做了“QML 没有 URL 构造器”的先验推理，未实跑。**该缺陷不存在**，本地 JSON 导入路径正常。

（顺带确认：全目录 `grep` 后 `new URL` 仅此一处，也没有 `document` / `window` / `fetch` / `localStorage` 用法；`qmllint` 亦未对此报警。）

### 2.2 ~~C-1（sbtui 用默认 current-thread runtime，退出固定卡 3 秒且跳过有序回收）~~ → **根因推翻，降级为 Low**【实测 + 源码确认】

上一版与 S5 子代理均称 `crates/sbtui/src/main.rs:6` 的 `#[tokio::main]` 默认是 current-thread。

源码反证（`tokio-macros 2.7.2`，registry 内实际解析到的版本）：

- `src/lib.rs`：`pub fn main(...) { entry::main(args.into(), item.into(), /* rt_multi_thread = */ true) }`；只有 `main_rt` 才传 `false`，而 `main_rt` 仅在 tokio 只开了 `rt`、没开 `rt-multi-thread` 时被导出。
- `src/entry.rs`：`default_flavor: match is_test { true => CurrentThread, false => Threaded }`，且 `Threaded` 在缺少 `rt-multi-thread` 时是**编译期报错**，不会静默降级。
- 官方文档原文：`The worker_threads option ... defaults to the number of cpus on the system. This is the default flavor.`，以及 `The default test runtime is single-threaded.`

`crates/sbtui/Cargo.toml:16` 启用了 `rt-multi-thread`，因此：

- **生产**（`main.rs:6`、`bin/ly.rs:6`）：`#[tokio::main]` = **multi_thread**，`shutdown()` 里的 `std::thread::sleep` 只占用一个 worker，引擎任务可在其它 worker 上被轮询 → 有序回收**会**执行，通常提前返回。
- **测试**（`#[tokio::test]`）：默认 current_thread → 子代理观测到的“单个测试 3.07s”正是**测试夹具**的开销，不是产品行为。子代理把方向搞反了。

**残留的真实风险（Low）**：`controller.rs:138-156` 在 async 上下文里用阻塞 sleep 轮询，正确性隐式依赖 `worker_threads >= 2`。默认 worker 数 = CPU 数，因此在**单核机器 / 被 cgroup 限制为 1 CPU 的环境**上仍会退化为“卡满 `SHUTDOWN_GRACE = 3s` 且跳过有序回收”，随后 sing-box 靠 `kill_on_drop` 被强杀。建议显式 `#[tokio::main(flavor = "multi_thread", worker_threads = 2)]`，或把 `shutdown()` 改成 async/`block_in_place`。

### 2.3 ~~D2（v0.1.26 仍由公开开发密钥签名且无 install.sh）~~ → **推翻**【子代理 + 代码确认】

- **S6 子代理**用 `gh api` 取到：远端 tag 只剩 `v0.0.4 / v0.0.3 / v0.0.2 / v0.0.1`，`gh release list` 无 v0.1.26；现行最新发布 v0.0.4 的 `package` 作业使用 `secrets.SBCTL_SIGNING_SEED`（`release.yml:329/344`），并调用 `scripts/generate-manifest.sh:29`（该行已去掉开发密钥回退：`signing_key=${SBCTL_SIGNING_KEY:?...}`）与 `release verify` 自检（`:57`）；若 seed 是开发密钥，编译期就会被 `src/release.rs:35` 拒绝。v0.0.4 资产中包含 `install.sh`。〔**未复核**：主线所在网络无法访问 GitHub API，无法独立复核远端事实。〕
- 主线用 git 本地事实确认了**同一结论的另一半**：`v0.1.26` 的 `release.yml` 只上传 `sbctl-linux-*` / `sing-box-linux-*`（上传清单里没有客户端、没有 MSI、没有 install.sh），而 `v0.0.4` 的 `release.yml` 同时生成了 `install.sh` 与 MSI 清单——**版本治理确实在 v0.1.26 之后才补齐**。

因此该条的准确表述是：**“无 install.sh”这一事实成立于 v0.1.26（一个已被删除的历史 tag），而不成立于现行发布 v0.0.4。**

### 2.4 ~~“最新 tag = v0.1.26”~~ → **推翻**【实测】

```
$ git for-each-ref --sort=-creatordate --format='%(refname:short) %(creatordate:iso8601)' refs/tags | head -5
v0.0.4 2026-09-26 13:47:24 +0800
v0.0.3 2026-09-26 13:39:54 +0800
v0.0.2 2026-09-26 12:48:19 +0800
v0.0.1 2026-09-26 03:21:21 +0800
v0.1.26 2026-09-13 23:04:51 +0800
```

并且 `git merge-base --is-ancestor v0.1.26 v0.0.4` 为真——**v0.0.4 是 v0.1.26 的后代**。也就是说版本号从 `0.1.26` **降级**回 `0.0.1…0.0.4`：既有发布链读作 `v0.1.26 → v0.0.4`，这是本轮发现的最严重的发布治理问题，比上一版描述的“版本号四分五裂”更严重（不是数值不一致，而是**版本序列倒退**）。

### 2.5 需修正的其他条目

| 上一版条目 | 修正后 |
|---|---|
| M7 IPv6 VMess 括号 | 结论成立，行号应指 `src/subscription/render/uri.rs:25`（写入 add）与 `:262-265`（`with_bracketed_host`），不是 92-121 |
| M8 Direct 模式忽略 `--bind` | 结论成立，真实路径是 `src/subscription/serve.rs:37-39`（不是 `cli/commands/serve.rs`）；`cli/commands/serve.rs:27` 的 `"0.0.0.0:0"` 是死代码 |
| M9 TOCTOU | 结论成立，行号 593-595（读+校验）与 596/606（按路径执行）；**触发前提收窄**：需 `--sbctl-artifact/--sing-box-artifact` 指向他人可写路径，默认下载路径不可触发 → Medium |
| M18 YAML `<<` 不展开 | 不展开确认；但“静默产出错误配置”的机制应改为：字面 `<<:` 在**下游** yaml.v3 里会按“显式键优先”展开，效果是**覆写与生成基底同名的键被基底压掉**（覆写静默失效），而非整份配置错乱。另：client-core 未启用 json-merge 的 `yaml` feature，该路径只影响服务端 |
| G-3 adapter 闩死 | 闩死代码存在，但 label 不匹配在现引擎**不可达**；真正可达的是“引擎以 `OperationAlreadyRunning` 拒绝时不发任何生命周期事件”（`controller.rs:326-334`），请求永久留在 `adapter.pending`，之后同 key 命令一律静默返回 `Ok(false)` |
| G-4 12 个模型全量重建 | 投影层成立（GUI 线程 `reproject` 12 个模型 + 整表比较），但 Qt 模型侧只对**变化**模型做增量替换（`bridge.rs:1167-1173`）；O(n²) 行差成立（`bridge.rs:886-910`） |
| C-4 文件 0644 | 成立且敏感度排序明确：订阅缓存（含 uuid/password）> `profiles.toml`（含带凭据订阅 URL）> `core.log` > `settings.toml`。同仓 `active-config.json`（`core.rs:84-92`）与 override（`config_override.rs:465-469`）已显式 0600，说明是遗漏而非统一决策 |
| D1 release-signing 文档漂移 | 文档行号精确（`docs/release-signing.md:37`），但“客户端参与发布”目前只对 **HEAD** 成立：三个真实 release 都只有 7 个服务端资产，客户端/MSI 的 job 是 v0.0.4 之后新增、**从未运行过** |
| D3 版本号四分五裂 | 数字全部核实；但“最新 tag”按 §2.4 修正为 v0.0.4，且额外发现四个 crate 均为 `0.1.0`、`Cargo.lock` 与根包一致（锁文件无漂移） |
| D5 flaky 测试路径 | 真实路径为 `.scratch/subscription-capability-20261002/issues/28-windows-concurrent-read-test-flake.md`，状态 `needs-triage`；失败点已漂移到 `src/config.rs:1837`（文档仍引 1750） |

---

## 3. 高优先级问题（High）

### H-1 更新回滚用“兜底属主/权限”重写全部受管文件，破坏私钥组读与 deploy hook 可执行位

**位置**：`src/update.rs:695-710` + `src/config.rs:1613-1635`（回滚集合见 `src/update.rs:12-28`、`:652-666`）
**证据**：【代码确认】+【子代理】

`restore()` 对每条备份记录调用 `store.write_relative_locked(...)`，而该方法内部是：

```rust
// src/config.rs:1442-1450
atomic_write(&path, contents)?;
enforce_live_file_owner(&self.root, &path)
```

`enforce_live_file_owner` 的路径→属主映射只有三个分支：

```rust
// src/config.rs:1627-1635
"etc/sing-box/config.json" => ("sing-box", "sing-box:sing-box", 0o640),
"usr/local/bin/sbctl" | "usr/local/bin/sing-box" => ("root", "root:root", 0o755),
_ => ("sbctl", "sbctl:sbctl", 0o600u32),     // ← 兜底
```

但回滚集合远不止这三类：`MANAGED_PATHS` 含 5 个 systemd 单元与 `etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook`；`rollback_paths` 在 Direct 模式还追加 `var/lib/sbctl/certificates/<host>/{fullchain.pem,privkey.pem}`。

**后果**（回滚后系统状态比失败前更糟）：

1. **pinned 私钥被改成 `sbctl:sbctl 0600`**：`src/subscription/render/singbox.rs:495-505` 显示 Direct + domain 模式下 **sing-box 读的就是这份 pinned 副本**；正确的权限模型是 `src/certificate.rs:469-499` 的目录 `0750` + `chgrp -R sbctl-cert` + 文件组可读。改成 0600 后 sing-box 以 `sing-box` 账号读是 `EACCES`，回滚重启（`update.rs:571/629`）失败且只打印 warning；`grant_certificate_storage` 只做 chgrp 不做 chmod，**不会自动修复**，此后每次重启 sing-box 都起不来。
2. **certbot deploy hook 失去 `+x`**：该文件由 **root 的 certbot** 执行并回调 `sbctl certificate verify`（`src/lifecycle.rs:66`）。分路径：【子代理】**`sbctl update`** 的回滚随后会走 `restart_services → reconcile_certificate_hook`，那里无条件 `set_executable(0755)`（`lifecycle.rs:864`），x 位被修回；而 **`sbctl sing-box update`** 的回滚只调 `restart_sing_box_service`（`lifecycle.rs:148-151`），不做任何 unit/hook 协调 → **x 位永久丢失，certbot 续期后 hook 持续失败，pinned 证书不再刷新**。
3. systemd 单元被写成 `0600 sbctl:sbctl`：systemd 以 root 读取，无实际功能影响，属同一根因的噪声。

**修复**：在 `restore()` 按路径恢复正确策略（hook → `root:root 0755`；pinned 证书 → `root:sbctl-cert 0640` 并重新 chgrp；单元文件 → `root:root 0644`），并把 `install_candidate_sing_box` 的回滚改为能收敛 unit/hook 的完整 `restart_services`。更稳妥的做法是让 `enforce_live_file_owner` 覆盖全部受管路径而不是靠兜底。

### H-2 官方内核下载在 GitHub 摘要缺失时只警告，随后仍以 root 执行候选二进制

**位置**：`src/update.rs:322-402`（关键在 `:358-365` 与 `:395`）
**证据**：【代码确认】

```rust
// src/update.rs:358-365
if let Some(expected_digest) = expected_digest {
    verify_official_archive_checksum(archive.path(), &expected_digest)?;
} else {
    eprintln!("warning: GitHub 未提供 sing-box {} 资产的 SHA-256 摘要；将继续通过 HTTPS 下载并检查候选内核版本，但归档完整性未校验。", version);
}
```

随后 `confirm_sing_box_candidate`（`:406-414`）用 `Command::new(candidate).arg("version")` **以 root 执行该二进制**，`:399` 再落盘到 `/usr/local/bin/sing-box`。调用方为 `cli/commands/install.rs:183` 与 `cli/commands/update.rs:101`。

**影响**：供应链降级通道——MITM 或 GitHub 发布账号被入侵时，只要能影响 API 响应使 digest 字段缺失，即可在目标机以 root 执行任意二进制。“确认版本号”只校验 stdout，攻击者可伪造。

**修复**：digest 缺失时 **fail-closed**（拒绝安装并提示改用签名 manifest）；同时给 `tar -xzf`（`:369-373`）加成员路径校验 / `--no-same-owner`。

### H-3 安装失败回滚无条件删除 `/usr/local/bin/sbctl`，而它不属于本次事务、也不在任何备份里

**位置**：`src/lifecycle.rs:551-579`（列表项 `:559`）+ `:522-525` + `src/cli/commands/install.rs:249-251`
**证据**：【代码确认】

`rollback_fresh_installation` 的删除列表含 `"usr/local/bin/sbctl"`，而唯一的豁免判据 `predates_transaction` 只认 `etc/sbctl/config.toml` 与 `var/lib/sbctl` 前缀：

```rust
// src/lifecycle.rs:522-525
fn predates_transaction(relative: &str, preexisting: PreexistingState) -> bool {
    (preexisting.config && relative == "etc/sbctl/config.toml")
        || (preexisting.data_directory && relative.starts_with("var/lib/sbctl"))
}
```

而按发布流程，这个二进制**在事务开始前就存在**并被用来执行安装（`scripts/install.sh`）。`--replace-existing` 也救不回来：`src/preflight.rs:169-201` 的 conflict/归档列表不含 `usr/local/bin/sbctl`。

**影响**：一次失败安装移除用户唯一的 CLI（`status`/`uninstall`/`menu` 全部不可用，`ly` 变悬空软链），且 certbot deploy hook 调用的也是该路径 → Direct 模式证书固定环节一并失效。用户必须重新下载安装脚本。

**修复**：把 `usr/local/bin/sbctl` 纳入 `PreexistingState` 或归档范围；或仅在确认该文件是本事务写入时才删除（记录安装前 hash/存在性）。

### H-4 失败早于 `installation_started` 时，已写入的证书目录不回滚，下次安装被 preflight 拒绝

**位置**：`src/cli/commands/install.rs:195-208`（`installation_started = true` 在 `:208`）+ `:249-251`
**证据**：【代码确认】

自签证书在 `generated_artifacts_for_kernel → render/singbox.rs:493 certificate_tls_config` 的 `SelfSigned` 分支里由 `ensure_self_signed_certificate`（`:533-568`）生成，root=`/` 时写到**绝对路径** `/var/lib/sbctl/certificates/<sni>/{cert.pem,key.pem}`。默认配置就是 SelfSigned + 五协议，因此 `install.rs:206` 的 `check_sing_box_config` 一旦失败即触发本路径。

**影响**：残留的 `var/lib/sbctl/certificates` 让下一次安装被 `preflight.rs:199` 判为 `ExistingDeployment` 而拒绝，管理员必须手工 `rm -rf`。

**修复**：证书的写入点前移/登记进事务，或在 `installation_started` 之前失败时也执行一次受限清理。

### H-5【本轮新发现，实测】QML 声明式 `checked:` 绑定在用户第一次交互后即失效

**位置**：`crates/sbgui/qml/ShellControlBar.qml:246, :291, :386`；`crates/sbgui/qml/pages/OverridesPage.qml:212`；`crates/sbgui/qml/pages/ConnectionsPage.qml:628/635/642/649`；`crates/sbgui/qml/pages/LogsPage.qml:233`
**证据**：【实测】

用 `qmltestrunner`（Qt 6.10.3）写了一个判别性用例：控件 `checked` 绑定到 `root.model`，模拟真实鼠标点击后把 `model` 改成与当前 `checked` 相反的值，观察 `checked` 是否跟随。

```
PASS : SwitchBinding::test_imperative_assignment_destroys_binding()
FAIL!: SwitchBinding::test_user_click_and_binding()  Actual: true  Expected: false
        ... tst_switchbinding.qml(25) : failure location
Totals: 3 passed, 1 failed
```

两次运行结果一致。结论：

1. **命令式 JS 赋值会永久切断绑定**（对应 `ShellControlBar.qml:386` 的 `tunToggle.checked = ...`）。
2. **真实鼠标点击同样会切断绑定**——`AbstractButton::checked` 由控件自身在交互中写入，QML 会移除原有绑定。

这意味着**全仓 8 处 `checked:` 声明式绑定里有 7 处是“一次性”的**（唯一正确的是 `crates/sbgui/qml/pages/SettingsPage.qml:633-637`，它用了 `Binding { target: ...; property: "checked" }` 元素）：

| 位置 | 交互后表现 |
|---|---|
| `ShellControlBar.qml:246` 系统代理开关 | 首次点击后不再跟随 `store.systemProxyEnabled`；后端拒绝变更时开关仍显示新状态 |
| `ShellControlBar.qml:291` TUN 开关 | 首次点击后不再跟随；**另外**只要“内核运行中点 TUN → 弹窗取消”即被 `:386` 切断 |
| `OverridesPage.qml:212` 覆写片段开关 | 首次切换后不再跟随模型 |
| `ConnectionsPage.qml:628-649` 排序菜单（4 项） | 选择一次后旧项仍显示勾选 → 可能出现两项同时“已选” |
| `LogsPage.qml:233` 日志级别按钮 | 与 `ButtonGroup` 互斥叠加，视觉多半仍正确，但存储拒绝变更时会与后端脱钩 |

**触发条件**：任意一次用户点击/切换；`ShellControlBar.qml:386` 甚至只需取消一次弹窗。
**修复**：把 7 处 `checked:` 声明式绑定统一改成 `Binding { target: ...; property: "checked"; value: ... }`（照抄 `SettingsPage.qml` 的既有正确写法），并删除 `:386` 的 JS 赋值。
**测试缺口**：现有 QML 测试只覆盖初始渲染与按钮点击回调，没有一条断言“外部状态变化后控件跟随”，所以这个缺陷在 87 条 QML 测试里零告警。

### H-6 发布流水线对下载内容不做摘要校验，而签名 manifest 里的 sha256 是“下载后现算”的

**位置**：`.github/workflows/release.yml:305-318`（下载）、`scripts/generate-manifest.sh:36-37`（现算 sha256）
**证据**：【代码确认】+【子代理】

```yaml
# release.yml:313
curl --fail --location --silent --show-error "$url" --output "$archive"
# 316: 直接 install 成 sing-box-linux-${arch}
```

随后 `generate-manifest.sh` 对**刚下载的字节**计算 sha256 并签名。因此签名只证明“我收到了什么”，**不证明来源**；上游 release 资产被替换时，整条信任链会为被替换的字节背书。

同类问题：CI 内部下载内核亦无校验（`ci.yml:176-179`、`:222-224`、`:252-256`、`:309-313`），全仓 `sha256sum` 只用在 `generate-manifest.sh` 的“现算”。

**修复**：在 CI 中对上游资产**先**用独立渠道获得的摘要校验（或固定一份 sha256 清单作为仓库内常量），再落盘、再签名。

### H-7【子代理】`main` 领先 `origin/main` 86 个提交，CI 从未跑过，且当前 HEAD 过不了仓库自己的 fmt 闸门

**证据**：【实测】+【子代理】

```
$ git status -sb
## main...origin/main [ahead 86]

$ cargo fmt --all -- --check 2>&1 | grep -c '^Diff in'
40
```

`origin/main` 停在 `f9b9495 = v0.0.4`（2026-09-26），HEAD 为 `599572c`（2026-09-30）；`git diff --stat origin/main..HEAD` = 380 文件 / 63033 行新增，**其中包含 `.github/workflows/ci.yml` 与 `release.yml` 本身**。子代理补充：这批提交里的客户端/MSI 发布作业从未被 runner 执行过。

而 CI 的硬闸门 `cargo fmt --all -- --check`（`ci.yml:49`、`:107`）在 HEAD 上报告 40 处差异（本机 rustfmt 1.9.0-stable / rustc 1.97.1，与 CI 的 `dtolnay/rust-toolchain@stable` 同源）。也就是说：**这 86 个提交按现状推上去会直接红**。

**修复**：`cargo fmt --all` → 提交 → 推送，让 CI 真正跑一遍客户端与 MSI 发布作业；在此之前不要打任何 tag。

---

## 4. 中优先级问题

### 服务端事务 / CLI【代码确认 + 子代理】

| # | 位置 | 问题 |
|---|---|---|
| M-1 | `src/cli/commands/install.rs:11-18` | 非交互早退判据漏掉 `--mode/--http-port/--proxy-host/--protocol-sni/--disable-protocol/--*-port/--replace-existing/--no-start`。例如 `echo \| sbctl install --disable-protocol vmess` 会打印 `install preflight passed` 并 exit 0，**什么都没做**；`--replace-existing` 这种破坏性意图同样会空转。**注意**：`scripts/install.sh:142` 刻意依赖 `sbctl install </dev/null` 的只读预检语义，修复时必须保留 |
| M-2 | `src/cli/args.rs:264-270` + `src/cli/commands/config.rs:357-369` + `src/config.rs:798-810` | `config switch-mode --mode ip-fallback` **永远失败**：该子命令没有 `--http-port`，而 `http_port` 只在非 IpFallback 时被清空、切回来时不会被设置；且订阅主机在 Direct/ExternalProxy 下必为域名，与“必须是 IP”冲突。两个独立原因都让 `validate()` 失败 |
| M-3 | `src/cli/commands/config.rs:357-369` | 即使 `direct ↔ external-proxy` 切换成功，也只写 `config.toml`，**不做 unit 收敛/服务重启**：重启后 Direct 服务会因缺少 socket activation 进入崩溃循环，或 80/443 仍被残留的 `sbctl-http.socket` 占用 |
| M-4 | `src/subscription/artifacts.rs:84/179/270` vs `install` 事务 | `update`/`config`/`regenerate` 三类多文件事务都持 flock，但 **install 事务全程不持锁** |
| M-5 | `src/cli/args.rs:432-467` | `--format sing-box-9.9` 只做 `major.minor` 语法校验，不查注册表；服务端 `serve.rs:751-783` 才会 404 → CLI/`sub`/`qr` 输出**死链 URL**。`clash-<任意版本>` 同样被接受而服务端只认 1.18 |
| M-6 | `src/subscription/artifacts.rs:380-397` vs `profile.rs:332-359` / `index_page.rs:71` | AnyTLS-only 部署的工件里没有 `subscription-sing-box-1.10/1.11.json`，但总览页仍 4 次印出这两个链接，`read_authorized` 返回 NotFound → **503**；`sbctl sub` 无参矩阵与 `sbctl qr --all` 是同源的另外两个出口 |
| M-7 | `src/subscription/template.rs:348-354` + `render/clash.rs:196-234` | `global + minimal` 时孪生被写成 `Some(RuleMatcher::DomainSuffix(...))`，导致 `push_inline_cidrs(PRIVATE_ADDRESS_LIST)` 分支永不执行。**子代理实测**：`global+minimal` 的 clash 工件 `IP-CIDR` 计数为 **0**，而 `split/standard` 的 minimal 各带 17 条；sing-box 侧仍有 `ip_is_private`，故只有 Clash 格式被破坏，与文档“minimal 不改分流结果”矛盾 |
| M-8 | `src/index_page.rs:171` | 总览页已用流量写作 `traffic.received + traffic.transmitted`，忽略 `total_adjustment`；而 `TrafficReport::total()`（`traffic.rs:231-233`）走 `corrected_total`，`status --json` 同时输出两者。触发：`sbctl traffic set-used --bytes N` |
| M-9 | `src/update.rs:593-596/606` | 候选二进制“读后校验、按路径再执行”的 TOCTOU。安装写入用的是已校验缓冲，默认下载路径不可触发；需 `--*-artifact` 指向他人可写路径 |
| M-10 | `src/release.rs:55-63` + `update.rs:589` | 签名 manifest 无 `serial/issued_at/expires_at`，`apply` 不与已安装版本比较 → **不拒绝降级**（重放旧版攻击）。`install.sh:101-124` 同样只有 schema 与格式检查 |
| M-11 | `src/subscription/render/singbox.rs:541-546/599-604` | 自签私钥复用仅用 `is_file()`（跟随符号链接）判断存在即复用、**不校验 cert/key 配对**；写入用 `write(true).create(true).truncate(true)` 无 `create_new`/`O_NOFOLLOW`。前提成立：`var/lib/sbctl` 全树被 `chown -R sbctl:sbctl`（`lifecycle.rs:1212-1213`）。对照 `certificate.rs:439-443` 的正确写法 |
| M-12 | `src/subscription/serve.rs:198-200`、`:515-517` | 限流表超过 4096 时 `clear()`，且 `ExternalProxy` 完全不限流。凭据为 256 位随机值，在线暴破不可行 → 仅反滥用控制可被绕过（Low–Medium） |
| M-13 | `scripts/install.sh:83,125` | `curl` 未加 `--proto '=https'`，与 Rust 侧（`update.rs:90-107` 等强制 https，仅 loopback 例外）不一致；因 `:126` 有 sha256sum 校验，属纵深防御缺口 |
| M-14 | `src/subscription/serve.rs:37-39` | Direct 模式**静默忽略** `--bind`；ip-fallback 模式反而会真正采用且不做校验（可与 `config.http_port` 及已打印链接不一致） |

### client-core / json-merge【代码确认 + 子代理】

| # | 位置 | 问题 |
|---|---|---|
| C-1 | `crates/client-core/src/system_proxy.rs:119-161` | `restore_backup` 在备份不可读/不可解析时直接 `return Ok(())`（`:121-126`），且**不删除坏文件** → `has_residual_backup` 永远为真，`disable()` 永远走同一分支、永远清不掉真实代理。控制面据此把 `system_proxy_enabled` 置 false 并报“已关闭”（`controller.rs:849-850`）。启动残留清理（`controller.rs:217-218`）同样静默失效 |
| C-2 | `crates/client-core/src/system_proxy.rs:90-115` + `:156-161` | `capture_backup` 的写入失败被 `let _ =` 忽略，而 `enable()` 仍无条件 `set_proxy` → 一旦备份没写成，`disable()` 走“无备份即清空”分支会**抹掉用户原有代理**。该分支不是幂等操作，但 UI 退出路径会绕过引擎直接调用（`worker.rs:32` 的意外退出分支不看快照状态） |
| C-3 | `crates/client-core/src/controller.rs:832-835`、`:1533-1536` | 代理关闭失败仍把快照置为 `false`（对照 `toggle_system_proxy:849` 用了 `?`，属正确写法） |
| C-4 | `crates/client-core/src/settings.rs:86-91` | `Settings::save` 用 `fs::write` 直接截断写，**非原子、无备份、无恢复**；而 `profiles.toml` 已经是 `temp + sync_all + rename + 备份 + 启动恢复`（`:135-166`、`:210-241`）。后果：截断成空文件时全部字段带 serde default → **静默回默认值**；部分写 → 解析失败 |
| C-5 | `crates/client-core/src/controller.rs:202-208`、`:389-396` | 存储不可读时引擎“拒绝持久化”并**无解除途径**（设计上防丢数据，方向正确），但叠加 C-4 后会把用户锁在一个只能手工改文件的死状态 |
| C-6 | `settings.rs:228`、`settings.rs:89`、`controller.rs:940/1056`、`core.rs:309-315` | `profiles.toml`（含带凭据订阅 URL）、`settings.toml`、订阅缓存（含 uuid/password）、`core.log` 均未设置 mode → 默认 0644；`data_dir_for`（`settings.rs:298-302`）用 `create_dir_all` 建 0755 目录。同仓 `active-config.json` 与 override 已显式 0600 |
| C-7 | `crates/client-core/src/subscription.rs:351-371` | 同名节点重命名 O(k²)（nth 每次从 2 重扫）。放大因素：解析由 `controller.rs:939` 在 `finish_operation` 的 Completion 内**同步执行、跑在引擎单任务上** → 恶意订阅冻结的不只是导入，而是整个控制平面（含 shutdown 的 3s 宽限） |
| C-8 | `crates/client-core/src/core.rs:472-482`、`:593`、`:628` | 下载无大小上限；解压用归档头声明的 size 做 `Vec::with_capacity`（分配失败即 abort）；TOFU 台账（`:505-524`）非原子写、读取失败静默 `unwrap_or_default`（损坏即丢失全部记录），且 `:485` 在解压前就记为可信 |
| C-9 | 持久化路径（导入/改名/切换/删除） | 均**先改内存再 save**，save 失败后内存与磁盘分叉 |
| C-10 | `crates/sbgui/src/app.rs:961`、`crates/sbtui/src/lib.rs:148` | 在 `controller.shutdown()` **之前**调用代理 disable，与引擎的 `stop_core`/`schedule_restart` 存在竞态【未复核：需运行 GUI 测量】 |
| C-11 | `crates/client-core/src/system_proxy.rs:389-395` 等 | Windows 注册表 `get_value::<u32>` 类型不符即 `Err`（未实机验证概率）；macOS 未剥离 `networksetup` 服务名行首 `*`【未复核：无 macOS】 |
| C-12 | `crates/json-merge/src/lib.rs:138-170` + `src/override_template.rs:75-95` | YAML `<<` merge key 不展开（`serde_yaml` 的 `apply_merge()` 是显式 API，全仓零调用，见 registry `serde_yaml-0.9.34+deprecated/src/value/mod.rs:600-670`）。机制修正见 §2.5。另：`serde_yaml 0.9.34` 已被上游标记弃用（registry 目录名 `serde_yaml-0.9.34+deprecated`） |

### GUI 四端 / TUI【子代理 + 实测部分】

| # | 位置 | 问题 |
|---|---|---|
| G-1 | `crates/sbgui/src/qml/projection.rs:394-430`、`bridge.rs:1176-1189` | 真实 `ClientListModel` **没有任何属性**（依据本次构建产物 `target/debug/build/sbgui-*/out/.../Serein/Desktop/plugin.qmltypes`：`prototype: QAbstractListModel`、零 Property；Qt 6.10.3 的 `QAbstractItemModel` 也无 `count` Q_PROPERTY），而 `RulesPage.qml` 使用 `.count` → 生产环境为 `undefined`，**新入站分区永不显示**。测试之所以全绿，是因为 `scripts/sbgui-qml/qml-import-path.ps1:26-33` 把 `test-support/ClientListModel.qml`（全文 `ListModel {}`，**有** `count`）注入 staging 模块并追加进 `qmldir`，四个 harness 全部用假模型【未复核：需运行真实 GUI 观察】 |
| G-2 | `crates/sbgui/src/qml/adapter.rs:756-859` + `controller.rs:326-334` | 引擎以 `OperationAlreadyRunning` 拒绝时**不发任何生命周期事件**，请求永久留在 `adapter.pending`，之后同 key 命令一律静默 `Ok(false)`；QML 丢弃返回值（`SettingsPage.qml:290-295`、`ShellControlBar.qml:379-383`）。触发：后台操作与确认弹窗重叠 |
| G-3 | `crates/sbgui/src/qml/bridge.rs:886-910` | 行差 O(n²)：线性查找 + `Vec` remove/insert + 整行深拷贝；连接列表无上限（日志已截到 240 行） |
| G-4 | 四端漂移（子代理整理出 7 处） | 字节单位差异不在 QML 页面而在 `crates/sbgui/src/qml/projection.rs:784-797` 的本地 `format_bytes`（同文件 `:102-110` 用 `human_bytes`）；“概览的订阅与事件卡片”只有 Slint 有（`view/dashboard.rs:481` + `ui/pages/dashboard.slint:1036-1115`），订阅列表的用量四端都有；英文文案不一致实例：`System Proxy` vs `System proxy`、`Restore and quit` vs `Disable proxy and quit` |
| G-5 | `scripts/sbgui-qml/static-checks.ps1:34` | `qmllint --max-warnings 1000` —— 门禁**最多容忍 1000 条告警**。**主线实测**：当前全量 48 个 QML 文件只有 **2 条**告警，且都是真问题（`ProxiesPage.qml:577`、`:624` 在布局管理的 item 上设 `width`，属未定义行为）。所以真正的问题不是“放行 1000 条”，而是“这 2 条真告警被忽略”＋“qmllint 发现不了 `new URL`/`.count` 这类问题” |
| G-6 | `crates/sbtui/src/main.rs:6` | 阻塞式 `shutdown()` 的正确性隐式依赖 `worker_threads >= 2`；单核环境退化为卡满 3s 且跳过有序回收（详见 §2.2） |
| G-7 | QML 行为测试体系 | 测试全部跑在假 store（`test-support/ClientStore.qml`）之上，**没有任何一条断言 QML 与真实 Rust 桥接面的属性/方法契约**。H-5 与 G-1 两个生产缺陷都是从这个缺口漏过去的 |

### 测试 / 文档 / CI / 仓库卫生【子代理 + 实测】

| # | 位置 | 问题 |
|---|---|---|
| T-1 | 版本号 | 根 `Cargo.toml:3` = `0.0.4`；`Cargo.lock:7168` = `0.0.4`（一致）；四个 crate `Cargo.toml:3` = `0.1.0`；`packaging/windows/sbtui.wxs:17` = `0.2.0`；多份文档写 `0.2.0`（`docs/known-gaps-after-merge.md:4`、`docs/verification-and-build-flow.md:3`、`docs/dev-sbctl-server-validation-plan.md:36`）。叠加 §2.4 的版本**倒退**，注册表/MSI 版本与产品版本无单一来源 |
| T-2 | `.github/workflows/*` | 60 处 `uses:` **无一带 commit SHA**（`actions/checkout@v4`、`dtolnay/rust-toolchain@stable`、`actions/upload-artifact@v4`、`actions/download-artifact@v4`、`actions/setup-node@v4`、`jurplel/install-qt-action@v4`）。`install-qt-action@v4` 拉取的 Qt DLL 会进入发布工件，浮动 tag 的风险是实打实的数据通路。CI 权限本身是最小化的（`ci.yml:14-15`、`release.yml:17-21` 均 `contents: read`，仅 `package` 提权 + `environment: release`） |
| T-3 | Windows 门禁 | `cli` 目标声明 `required-features = ["test-signing"]`（`Cargo.toml:15-18`），而 `test-signing` 只在三处 Ubuntu 位置启用（`ci.yml:54/74`、`release.yml:256`）；Windows job 只做 fmt/clippy/`cargo test --workspace --lib`（`ci.yml:110`）→ **CLI 集成测试在 Windows 零覆盖**，且 Windows 上的 `#[cfg(windows)]` 夹具（`tests/cli/fixture.rs:130`、`:352`）永远编不到 |
| T-4 | flaky 未修 | `.scratch/subscription-capability-20261002/issues/28-windows-concurrent-read-test-flake.md:3` 仍是 `needs-triage`；失败点在 `src/config.rs:1837`（文档引用的 1750 已漂移），且该用例落在 Windows 门禁内 |
| T-5 | `docs/release-signing.md:37` | 文档称客户端不参与服务端 Release，而 HEAD 的 `release.yml:369-373` 明确上传全部客户端 + MSI + `install.ps1`。当前仍只是“文档 vs 即将生效的代码”矛盾（这些 job 从未运行过） |
| T-6 | 文档漂移 | `docs/installation.md:48` 写端口 `> 1024`，实际协议端口为 **10000–65535**（`config.rs:26-27`、`:683-688`），`> 1024` 只对订阅监听端口成立；`docs/subscription-guide.md:118` 称 `sbctl restart` 会重新生成订阅，实际 `cli/commands/config.rs:12-35` 的 `restart()` 只 check 现有配置后重启服务，真正的生成入口是 `sbctl regenerate` |
| T-7 | 仓库卫生 | `.scratch` 追踪 **285** 个文件 / 5.68 MB（其中 60 个 PNG 占 4.96 MB）；`prototypes` 追踪 **29** 个文件 / 4.52 MB（4 个 PNG 占 4.17 MB，最大 `prototypes/sbgui-progressive-workspace/design-comparison.png` 1.81 MB）；**无 LICENSE 正文**（`git ls-files` 只命中 vendored 的 `crates/sbgui/vendor/annotate-snippets/LICENSE-*`，`Cargo.toml:5` 却声明 `MIT OR Apache-2.0`）；`.git` 74 MB，`git count-objects -v` 报 **garbage: 11 / size-garbage: 2918 KB**（11 个 `tmp_obj_*` 残file），`git fsck` 另有 3268 个 dangling blob |
| T-8 | 仓库卫生（本轮新增） | **注册了 21 个 git worktree**，其中 18 个在仓库外（`C:\Users\ranly\.codex\worktrees\**` 17 个 + `.kilo/worktrees/oxidized-handle`），大量处于 detached HEAD；`.kilo/worktrees/oxidized-handle/Cargo.toml` 里还留着一份 `version = "0.2.0"` 的旧树，是版本号混乱的一个额外来源。仓库根另有未跟踪文件 `.scratch-review.diff` |
| T-9 | `release.yml:364-366` | `gh release upload` 刻意不加 `--clobber`（保护已发布字节的不可变性，意图正确），但重跑会失败 → **失败到一半的发布无法原地修复** |
| T-10 | 子代理不确定项 | QML 测试替身与真实注册类型同名时谁在 `import Serein.Desktop` 下胜出，子代理未能实跑验证（无论谁胜出，替身存在且无契约测试这一点是确定的） |

---

## 5. 低优先级 / 信息级

- `sbctl sing-box install` 绕过 check/health/rollback 事务（子代理 F-11）；若定位为纯离线夹具工具可降级为 Info。
- external-proxy / ip-fallback 的订阅监听端口在 install 前从不探测（子代理 F-10）。
- `src/subscription/serve.rs` 的限额参数（16 KiB 头 / 5s 头超时 / 30s 连接上限 / 32 并发信号量）与注释一致；`Connection: close` 正常。
- 旧内核（1.10/1.11）是否会因 override 注入 `action: reject` 而拒载：子代理只有注册表元数据（`profile.rs:150 route_rule_actions=false`）与推理，confidence medium【未复核】。
- IPC 端点：`secret` 由 getrandom 生成、仅经 0600 的 `cache/active-config.json` 传递、不走 argv；`clash_api` 客户端 `no_proxy()` 且把 `Authorization` 标记为 sensitive；控制端口用 `127.0.0.1:0` 现绑——这些做得对，记录在案以免后续重构破坏。
- 覆写保留字段策略（`RESERVED_POINTERS`：`clash_api`/`inbounds`/`auto_detect_interface` 不可被覆写夺走）方向正确。

---

## 6. 值得肯定的设计（不吝赞美）

1. **单节点模型贯穿三格式**：`client_outbounds`（`render/mod.rs:49-108`）是唯一来源，Sing-box/Clash/URI 三格式逐字段对齐，IPv6 只在 URI authority 加方括号（`canonical.rs:28-33`，有测试锁定）；VLESS Reality 公钥在生成期从私钥重推，避免历史脏数据漂移。
2. **签名信任链 fail-closed**：编译期 `option_env!` 锚点、硬拒公开开发密钥、签名覆盖除 `signature` 外全部字段、canonical JSON 自排序、**先验签再信任 URL/摘要**；`install.sh` 用等价的 `jq -S -c 'del(.signature)'` + `openssl pkeyutl` 复现同一策略。`prepare-installer.py:11` 再拒一次开发公钥。
3. **更新事务与原子写**：单文件写入是 `0600 临时文件 + write_all + sync_all + rename`（`config.rs:1524-1551`）；回滚点 `root-only 0600`、限代际数量（`update.rs:737-761`）；安装的提交点是 ownership marker，只在**三次 1.1s 采样**的观察窗通过后才写（`install.rs:231-236` + `lifecycle.rs:110-117/216-231`），能抓住 `Type=simple` 的秒级崩溃。
4. **HTTP 订阅端点安全基线**：路径凭据 + 常量时间比较（长度无关循环）+ 统一 404 + 拒绝查询串 + 16 KiB 头/5s/30s/32 并发/令牌桶 + 日志脱敏 + 有意不信任 `X-Forwarded-For` + systemd 最小权限加固。
5. **client-core 进程生命周期**：Linux 在 `fork/exec` 之间 `pre_exec` arm `PR_SET_PDEATHSIG` 并用 `getppid()` 关掉竞态窗口；Windows 用 `KILL_ON_JOB_CLOSE` 匿名 job object；macOS 诚实返回 false 让调用方报 `OrphanGuardWarning`；`tests/orphan_guard.rs` 是真正可证伪的 `kill -9` 双进程测试。启动用 `127.0.0.1:0` 现绑端口，避免控制到别人的实例。
6. **`profiles.toml` 的持久化是正确范本**：`temp + sync_all + rename + 备份 + 启动恢复`，并有“保存失败保留旧文件”“备份可恢复”两个失败路径测试（`settings.rs:456-511`）。`Settings::save`（C-4）与它是同一个文件里的对照，修复成本极低——照抄即可。
7. **测试是真实黑盒**：限流不可区分性、凭据不泄漏、订阅读取不改 state、容器里跑 Debian 12/Ubuntu 22.04/24.04 三发行版的生产工件、`test-signing` 构建单独挂在另一条路径（`tests/acceptance/run.sh:7-9`）、`release_trust` 在 `--no-default-features` 下证明普通构建拒收开发密钥签名。
8. **CI 权限最小化**：默认 `contents: read`，只有 `package` 提权且带 `environment: release`，注释明确写出“构建 job 持有不可信输入，不得重写 release”；`workflow_dispatch` 会拒绝 tag 与 HEAD 不一致的发布。

---

## 7. 发布前修复清单

**P0（打 tag 之前必须完成）**

1. **H-1** 回滚权限映射按路径分派（deploy hook `0755`、pinned 证书 `root:sbctl-cert 0640`+chgrp、单元 `0644`），并把 `sing-box update` 的回滚换成完整 `restart_services`。
2. **H-2** 官方内核 digest 缺失时 **fail-closed**；`tar` 加成员路径校验。
3. **H-3 / H-4** 安装回滚不再删非本事务写入的 `usr/local/bin/sbctl`；证书目录要么前移登记进事务、要么在早失败时清理。
4. **H-5** 7 处 `checked:` 声明式绑定改成 `Binding` 元素，删除 `ShellControlBar.qml:386` 的 JS 赋值；补一条“外部状态变化后控件跟随”的 QML 测试。
5. **H-6** CI 下载链加独立 sha256 校验（先校验、后落盘、后签名）。
6. **H-7 / T-1** `cargo fmt --all` 修正 → 推送 → 让 CI 真跑一遍客户端/MSI 发布作业；版本号收敛到单一来源（建议由 workspace version 驱动 wxs 与文档）。
7. **T-2** 第三方 action pin 到 commit SHA。
8. **M-1** 非交互早退判据补齐（同时保留 `install.sh:142` 依赖的只读预检语义）。

**P1**

9. **M-2/M-3** `switch-mode` 增加 `--http-port` 并做 unit 收敛/服务重启，或直接下线 ip-fallback 切换。
10. **C-1/C-2/C-3** 系统代理：坏备份必须显式报错并清理；备份写入失败不得设置代理；关闭失败不得把快照置 false。
11. **C-4/C-5** `Settings::save` 改成 `profiles.toml` 同款原子写；给“拒绝持久化”状态一个可在 UI 内解除的恢复入口。
12. **C-6** 凭据文件与数据目录补 `0600`/`0700`。
13. **G-1** 给真实 `ClientListModel` 加 `count` 属性（或用 `rowCount()`/`ListView.count` 语义替代），并补一条针对**真实**桥接面属性的契约测试。
14. **T-3** Windows job 补 `test-signing` 下的 CLI 集成覆盖，或明确记录该缺口。
15. **T-4** 修 flaky 用例并更新 `.scratch` 里的定位。

**P2**

16. **M-4** install 事务持 flock。
17. **M-5/M-6** CLI 格式名走注册表校验；总览页/`sub`/`qr` 的链接按实际生成的工件集合渲染。
18. **M-7** `global+minimal` 的 Clash 孪生补回私有 CIDR（补一条断言 `IP-CIDR` 数量的测试）。
19. **M-8** 总览页改用 `traffic.total()`。
20. **M-10** manifest 增加 `serial`/有效期并在 apply 时拒绝降级。
21. **C-7/C-8** 订阅解析移出引擎单任务（或加上限 + 迭代式去重）；下载/解压加大小上限；TOFU 台账原子写。
22. **G-2/G-3** adapter 在 `OperationAlreadyRunning` 时也要清 pending 并向 QML 报错；行差改用索引映射。
23. **G-5** `--max-warnings` 降到 0 并修掉 `ProxiesPage.qml:577/624`。
24. **C-12** `deep_merge_yaml` 先调 `apply_merge()`；评估迁移到维护中的 YAML 库。

**P3**

25. **T-5/T-6** 文档漂移批量修订。
26. **T-7/T-8** `.scratch` 与 `prototypes` 大图出库（或改 Git LFS）、补 LICENSE 正文、`git gc` 清 11 个 garbage 对象、清理 18 个仓库外 worktree 注册与 `.scratch-review.diff`。
27. **T-9** 为发布的失败重跑提供显式的受控恢复流程。
28. **G-6/M-13/M-14** 显式指定 tokio flavor/worker 数、`install.sh` 补 `--proto '=https'`、Direct 模式的 `--bind` 要么生效要么告警。

---

## 8. 逐条对照表（上一版 → 本轮）

| 上一版 | 本轮判定 | 依据 |
|---|---|---|
| S-1 回滚权限映射 | **confirmed**（deploy hook 影响需分路径） | 代码确认 + 子代理 |
| S-2 digest 缺失仍 root 执行 | **confirmed** | 代码确认 |
| S-3 test-signing 信任锚 | **confirmed**（降为 Medium：非默认 feature，仅验收夹具用） | 代码确认 + 子代理 |
| S-4 安装回滚删 CLI | **confirmed** | 代码确认 |
| S-5 证书目录不回滚 | **confirmed** | 代码确认 |
| C-1 sbtui current-thread | **refuted**（生产是 multi_thread；3s 是测试夹具开销） | tokio-macros 源码 + 官方文档 |
| C-2 代理备份损坏静默 no-op | **confirmed** | 代码确认 |
| C-3 settings.toml 非原子写 | **confirmed** | 代码确认 |
| C-4 凭据文件 0644 | **confirmed** | 代码确认 |
| G-1 QML `new URL` 不可用 | **refuted**（Qt 6.10.3 实测可用） | 实测 |
| G-2 TUN 开关绑定被切断 | **confirmed**，并**扩展**为 7/8 处绑定系统性失效 | 实测 + 代码确认 |
| G-3 adapter 闩死 | **shifted**（真正可达路径是 `OperationAlreadyRunning` 无事件） | 子代理 |
| G-4 12 模型全量重建 | **shifted**（投影全量、Qt 侧增量；O(n²) 成立） | 子代理 |
| G-5 四端漂移 | **confirmed**（单位差异位置修正） | 子代理 |
| G-6 假 store + 1000 warning | **一半 confirmed**：假 store 确认且**掩盖了生产缺陷**；qmllint 实测只有 2 条告警 | 实测 + 子代理 |
| M1 非交互早退漏参数 | **confirmed**（并补充 `--replace-existing` 等） | 代码确认 |
| M2 switch-mode ip-fallback 必失败 | **confirmed** | 代码确认 |
| M3 总览页忽略 total_adjustment | **confirmed** | 代码确认 |
| M4 global+minimal 私有 IP | **confirmed**（子代理实测 IP-CIDR 计数 0 vs 17） | 子代理 |
| M5 AnyTLS-only 死链 | **confirmed** | 子代理 |
| M6 `--format` 不查注册表 | **confirmed** | 子代理 |
| M7 IPv6 VMess 括号 | **confirmed**（行号修正） | 子代理 |
| M8 Direct 忽略 `--bind` | **shifted**（路径修正） | 子代理 |
| M9 TOCTOU | **confirmed**（触发前提收窄，Medium） | 子代理 |
| M10 manifest 无降级保护 | **confirmed** | 子代理 |
| M11 自签私钥复用 | **confirmed** | 子代理 |
| M12 限流 clear + ExternalProxy | **confirmed**（凭据 256 位，降为 Low） | 子代理 |
| M13 install.sh 无 `--proto` | **confirmed** | 代码确认 |
| M18 YAML `<<` | **shifted**（覆写静默失效，非配置错乱） | 子代理 + 源码 |
| D1 release 文档漂移 | **shifted**（矛盾尚未成为既成事实） | 子代理 |
| D2 v0.1.26 开发密钥 + 无 install.sh | **refuted**（该 tag 已被删除；现行 v0.0.4 由生产 seed 签名且含 install.sh） | 子代理【未复核远端】+ 代码确认 |
| D3 版本号四分五裂 | **shifted**（最新 tag 是 v0.0.4，且版本序列**倒退**） | 实测 + 子代理 |
| D4 CI 无摘要/浮动 tag/Windows 无 test-signing | **confirmed**（三点全部代码确认） | 代码确认 + 子代理 |
| D5 flaky 未修 | **confirmed**（路径与行号修正） | 子代理 |
| D6 .scratch/prototypes 体积、无 LICENSE | **confirmed**（主线独立复核了全部数字） | 实测 |
| D7 自述 fmt/clippy 未过 | **confirmed**（主线实测 40 处 fmt 差异） | 实测 |
| D8 27 个改动游离于 CI | **confirmed**，且范围扩大到 86 个未推送提交 | 实测 |

---

## 9. 附录：本轮用到的可复现证据

```powershell
# 1) 版本/tag 事实
git for-each-ref --sort=-creatordate --format='%(refname:short) %(creatordate:iso8601)' refs/tags
git merge-base --is-ancestor v0.1.26 v0.0.4    # 返回 0 => v0.0.4 是后代
git status -sb                                  # ## main...origin/main [ahead 86]

# 2) 仓库自己的 fmt 闸门
cargo fmt --all -- --check                      # 40 处 Diff in ...

# 3) 仓库卫生
(git ls-files .scratch).Count                   # 285
(git ls-files prototypes).Count                 # 29
git count-objects -v                            # garbage: 11, size-garbage: 2918
git ls-files | Select-String 'LICENSE|COPYING|NOTICE'   # 只命中 vendored

# 4) QML 静态门禁（需 QT_ROOT_DIR）
$env:QT_ROOT_DIR = "C:\Qt\6.10.3\msvc2022_64"
pwsh -File scripts\sbgui-qml\static-checks.ps1  # 48 files, 仅 2 条告警, exit 0

# 5) 推翻 G-1 的探针
$env:QT_QPA_PLATFORM = "offscreen"
qml.exe <probe>.qml        # typeof URL -> function; localFilePath -> C:/tmp/a b.json

# 6) 证实 H-5 的判别性用例
qmltestrunner.exe -input <tst_switchbinding> -import C:\Qt\6.10.3\msvc2022_64\qml -platform offscreen
#   PASS : test_imperative_assignment_destroys_binding
#   FAIL!: test_user_click_and_binding   Actual: true  Expected: false

# 7) 推翻 C-1 的源码依据
#   ~/.cargo/registry/src/.../tokio-macros-2.7.2/src/lib.rs
#     pub fn main(..) { entry::main(args.into(), item.into(), true) }   # rt_multi_thread = true
#   .../src/entry.rs
#     default_flavor: match is_test { true => CurrentThread, false => Threaded }
```

---

**报告结束。** 本轮最该带走的三件事：① 上一版的 G-1 与 C-1 是**误报**，不应进入修复清单；② QML 开关绑定失效是**新发现的、可 100% 复现的**生产缺陷，且被测试替身完整掩盖；③ 当前 HEAD **过不了仓库自己最基础的 CI 闸门**，在这一条修好之前，任何"项目已就绪"的结论都不成立。
