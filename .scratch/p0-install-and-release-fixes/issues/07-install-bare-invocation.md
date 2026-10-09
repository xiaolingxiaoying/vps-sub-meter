# 07 `sbctl install` 早退判据覆盖全部参数

Status: ready-for-agent

## 问题

`src/cli/commands/install.rs::install` 的只读预检早退条件只看六个字段，因此
`--disable-protocol`、`--mode`、`--http-port`、`--proxy-host`、`--protocol-sni`、
五个 `--*-port`、`--replace-existing`、`--no-start`、`--manage-firewall`、`--ipv4-only`
在非 TTY 下会打印 `install preflight passed` 并 exit 0，什么都没做。`--replace-existing`
这种破坏性意图静默空转尤其危险。

同时 `scripts/install.sh` 依赖"裸调用 + 非 TTY = 只读预检"，该语义必须保留。

## 要做

- 新增 `InstallOptions::is_bare()`：用 `let Self { .. } = self;` 解构全部字段，新增字段
  会成为编译错误；判据为 `mode == Direct` 且 `!guided` 且所有 `Option` 为 `None`、
  所有 `Vec` 为空、所有 `bool` 为 `false`。
- 早退条件改为 `options.is_bare() && !io::stdin().is_terminal()`。
- `--guided` 在非 TTY 下仍走向导（读到 EOF 会明确报错）。

## 验收

- CLI 测试：对上述每个参数各写一例，断言非 TTY 下 stdout 不含 `install preflight passed`
  且退出码为 2、stderr 含 `is required`。
- 既有测试 `install_reports_a_supported_systemd_fixture_as_ready` 继续证明裸调用仍打印预检结果。

## 实现

`src/cli/args.rs`、`src/cli/commands/install.rs`、
`tests/cli/install.rs::install_arguments_that_carry_intent_never_report_a_preflight_as_success`。

## 验证命令

```bash
cargo test --features sbctl/test-signing --test cli -- install_arguments_that_carry_intent
cargo test --features sbctl/test-signing --test cli -- install_reports_a_supported_systemd_fixture_as_ready
```
