# 05 官方内核下载 fail-closed，且只解出预期归档成员

Status: ready-for-agent

## 问题

`src/update.rs::download_sing_box_official` 在 GitHub 未提供资产 `digest` 时只打印
warning 并继续：以 root `tar -xzf` 整个归档（无成员校验、无 `--no-same-owner`），
再以 root 执行候选二进制做"版本自检"，最后复制到 `/usr/local/bin/sing-box`。
这条路径是 `sbctl install`（不给 `--manifest`）与 `sbctl sing-box update` 的默认分支。

能影响 API 响应使 `digest` 缺失的一方即可在目标机以 root 执行任意二进制。

## 要做

- 新增 `UpdateError::OfficialChecksumUnavailable`；`require_official_archive_digest` 在
  摘要缺失时返回该错误，文案给出 `--manifest` 与 `--sing-box-bin` 两条替代路径。
- 新增 `official_archive_member(version)`（`sing-box-<version>-linux-<arch>/sing-box`）与
  `validate_archive_member`（拒绝空名、绝对路径、`..`、前导 `-`、内嵌 NUL）。
- `official_archive_members` 用 `tar -tzf` 列成员；拒绝任何危险名字，并要求预期成员存在。
- 解压只解出该成员，带 `--no-same-owner --no-same-permissions`。
- `docs/installation.md` 说明默认官方路径依赖上游摘要，缺失时改用签名清单。

## 验收

- 单测：摘要缺失 → `OfficialChecksumUnavailable`，文案含 `--manifest` 与 `--sing-box-bin`。
- 单测：危险成员名被拒绝；`LICENSE`/`README.md` 之类普通成员不误伤。
- 实测（2026-10-08）：用 sing-box 1.14.2 官方归档验证成员名为
  `sing-box-1.14.2-linux-amd64/sing-box`，只解出该成员后 `sing-box version` 与
  `sing-box check` 均通过（二进制 DT_NEEDED 只有 libc/libpthread/libdl，`libcronet.so`
  是按需 dlopen，不影响默认服务端配置）。

## 实现

`src/update.rs`、`docs/installation.md`。

## 验证命令

```bash
cargo test --lib -- update::tests::a_missing_upstream_digest_fails_closed_before_any_download
cargo test --lib -- update::tests::archive_member_names_that_tar_must_never_extract_as_root_are_rejected
```
