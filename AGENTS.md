## Project Overview

`vps-sub-meter` (aliases `vsm`, `ly`) is a streamlined dual-core (Xray + sing-box) PQC proxy and private subscription control plane written in Rust, paired with external config/templates and modular Bash operational scripts.

## Domain Docs

See `CONTEXT.md` at the repository root.

## Local Build & Test

- Format check: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Unit & integration tests: `cargo test --all-targets`
- Shell syntax check: `bash -n scripts/*.sh`

## 使用中文回答我
