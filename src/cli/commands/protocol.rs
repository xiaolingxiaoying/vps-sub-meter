use crate::cli::args::{PresetFormat, ProtocolCommand};
use sbctl::core::presets::{Node, validate_nodes};
use sbctl::core::xray::CertificatePaths;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

fn load(path: &Path) -> Result<Vec<Node>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "无法读取协议组合文件")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取协议组合文件")?;
    if bytes.len() > 1024 * 1024 {
        return Err("协议组合文件超过 1 MiB".into());
    }
    // Serde diagnostics may quote unknown user input, including credentials.
    let nodes: Vec<Node> = serde_json::from_slice(&bytes)
        .map_err(|_| "协议组合文件不是合法的节点 JSON 数组（请检查字段和预设名称）")?;
    validate_nodes(&nodes).map_err(|error| error.to_string())?;
    Ok(nodes)
}

fn render(
    nodes: &[Node],
    format: PresetFormat,
    certificate: Option<&CertificatePaths<'_>>,
) -> Result<String, String> {
    let render_error = |error: sbctl::core::presets::Error| error.to_string();
    let document = match format {
        PresetFormat::XrayServer => {
            json!({"log": {"loglevel": "warning"}, "inbounds": nodes.iter().map(|n| n.xray_inbound(certificate)).collect::<Result<Vec<_>,_>>().map_err(render_error)?, "outbounds": [{"protocol": "freedom", "tag": "direct"}]})
        }
        PresetFormat::XrayClient => {
            json!({"outbounds": nodes.iter().map(Node::xray_outbound).collect::<Result<Vec<_>,_>>().map_err(render_error)?})
        }
        PresetFormat::SingBox | PresetFormat::Mihomo | PresetFormat::MihomoXhttp => {
            let mut values = Vec::new();
            for node in nodes {
                if matches!(format, PresetFormat::MihomoXhttp) && node.preset.is_reality() {
                    eprintln!(
                        "跳过 Reality 节点：Mihomo 与当前 Xray 服务端基线存在 Reality 兼容边界"
                    );
                    continue;
                }
                // Conservative compatibility baseline until XHTTP has passed
                // the pinned Mihomo release and client subscription matrix.
                if node.preset.is_xhttp() && !matches!(format, PresetFormat::MihomoXhttp) {
                    eprintln!("跳过一个 XHTTP 节点：该导出目标尚未通过 XHTTP 兼容性验证");
                    continue;
                }
                values.push(
                    if matches!(format, PresetFormat::SingBox) {
                        node.sing_box_outbound()
                    } else {
                        node.mihomo_proxy(matches!(format, PresetFormat::MihomoXhttp))
                    }
                    .map_err(render_error)?,
                );
            }
            if values.is_empty() {
                return Err("目标格式没有兼容节点，未生成空订阅".into());
            }
            if matches!(format, PresetFormat::SingBox) {
                json!({"outbounds": values})
            } else {
                json!({"proxies": values})
            }
        }
        PresetFormat::Uri | PresetFormat::Shadowrocket => {
            let selected: Vec<_> = nodes
                .iter()
                .filter(|node| {
                    !matches!(format, PresetFormat::Shadowrocket) || !node.preset.is_xhttp()
                })
                .collect();
            if selected.len() != nodes.len() {
                eprintln!("Shadowrocket 兼容导出跳过尚未验证的 XHTTP 节点");
            }
            if selected.is_empty() {
                return Err("目标格式没有兼容节点，未生成空订阅".into());
            }
            return Ok(selected
                .iter()
                .map(|node| node.share_uri())
                .collect::<Result<Vec<_>, _>>()
                .map_err(render_error)?
                .join("\n")
                + "\n");
        }
    };
    if matches!(format, PresetFormat::Mihomo | PresetFormat::MihomoXhttp) {
        serde_yaml::to_string(&document).map_err(|_| "YAML 导出失败".into())
    } else {
        serde_json::to_string_pretty(&document).map_err(|_| "JSON 导出失败".into())
    }
}

pub(crate) fn run(root: &Path, command: ProtocolCommand) -> ExitCode {
    if let ProtocolCommand::Enable {
        input,
        sing_box_bin,
    } = &command
    {
        return enable(root, input, sing_box_bin.as_deref());
    }
    if matches!(command, ProtocolCommand::List) {
        println!(
            "vless-xhttp-reality  VLESS + XHTTP + Reality（Xray）\nvless-xhttp-tls      VLESS + XHTTP + TLS（Xray）\nvless-ws-tls         VLESS + WebSocket + TLS\ntrojan-tcp-tls       Trojan + TCP + TLS\n候选生成入口，不安装或激活服务。sing-box/Mihomo/Shadowrocket 当前保守导出仅含后两项；XHTTP 客户端兼容性待真核验证。"
        );
        return ExitCode::SUCCESS;
    }
    let (input, certificate, private_key, format, binary) = match command {
        ProtocolCommand::Check {
            input,
            certificate,
            private_key,
            xray_bin,
        } => (input, certificate, private_key, None, xray_bin),
        ProtocolCommand::Export {
            input,
            certificate,
            private_key,
            format,
        } => (input, certificate, private_key, Some(format), None),
        ProtocolCommand::List => unreachable!(),
        ProtocolCommand::Enable { .. } => unreachable!(),
    };
    let result = (|| -> Result<(), String> {
        let nodes = load(&input)?;
        let certificate = certificate
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let private_key = private_key
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let paths = certificate
            .as_ref()
            .zip(private_key.as_ref())
            .map(|(cert, key)| CertificatePaths {
                certificate: cert,
                private_key: key,
            });
        if let Some(format) = format {
            let output = render(&nodes, format, paths.as_ref())?;
            print!("{output}");
        } else if let Some(binary) = binary {
            let document = render(&nodes, PresetFormat::XrayServer, paths.as_ref())?;
            sbctl::core::CoreKind::Xray
                .check_config(&binary, &document)
                .map_err(|error| error.to_string())?;
            println!("协议组合和 Xray 服务端配置检查通过；未激活服务");
        } else {
            println!(
                "{} 个协议组合通过结构与凭据检查；未执行内核检查，未激活服务",
                nodes.len()
            );
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("协议组合操作失败：{error}");
            ExitCode::from(2)
        }
    }
}

fn enable(root: &Path, input: &Path, binary: Option<&Path>) -> ExitCode {
    let result = (|| -> Result<(), String> {
        let nodes = load(input)?;
        if root == Path::new("/") {
            let active_bytes = sbctl::config::read_managed_file_no_follow(Path::new(
                "/etc/sbctl-xray/config.json",
            ))
            .map_err(|_| "无法读取伴随内核配置")?;
            let active: serde_json::Value =
                serde_json::from_slice(&active_bytes).map_err(|_| "伴随内核配置无效")?;
            for node in &nodes {
                let inbound = active["inbounds"]
                    .as_array()
                    .and_then(|entries| {
                        entries
                            .iter()
                            .find(|entry| entry["tag"].as_str() == Some(&node.name))
                    })
                    .ok_or("伴随内核中缺少待发布节点")?;
                let cert =
                    inbound["streamSettings"]["tlsSettings"]["certificates"][0]["certificateFile"]
                        .as_str()
                        .unwrap_or("");
                let key = inbound["streamSettings"]["tlsSettings"]["certificates"][0]["keyFile"]
                    .as_str()
                    .unwrap_or("");
                let paths = CertificatePaths {
                    certificate: cert,
                    private_key: key,
                };
                if node.xray_inbound(Some(&paths)).map_err(|e| e.to_string())? != *inbound {
                    return Err("待发布节点与活动伴随内核配置不一致".into());
                }
            }
            let unit = std::fs::read_to_string("/etc/systemd/system/sbctl-xray.service")
                .map_err(|_| "先安装并验证 sbctl-xray 托管服务")?;
            if !unit.contains("Description=sbctl managed protocol companion")
                || !std::process::Command::new("systemctl")
                    .args(["is-active", "--quiet", "sbctl-xray.service"])
                    .status()
                    .map_err(|_| "无法检查伴随内核状态")?
                    .success()
            {
                return Err("sbctl-xray 托管服务未就绪，拒绝发布不可用节点".into());
            }
        }
        let store = sbctl::config::DeploymentStore::new(root);
        let _lock = store.acquire_operation_lock().map_err(|e| e.to_string())?;
        let path = root.join("etc/sbctl/protocol-presets.json");
        let previous = match sbctl::config::read_managed_file_no_follow(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("无法读取现有组合源文件".into()),
        };
        let bytes = serde_json::to_vec_pretty(&nodes).map_err(|_| "组合序列化失败")?;
        store
            .write_relative_locked("etc/sbctl/protocol-presets.json", &bytes)
            .map_err(|e| e.to_string())?;
        let managed = root.join("usr/local/bin/sing-box");
        let binary = binary.unwrap_or(&managed);
        let regeneration =
            sbctl::subscription::regenerate_current_locked(&store, Some(binary), false);
        match regeneration {
            Ok(config) => sbctl::lifecycle::prepare_daemon_storage(
                root,
                config.subscription_mode == sbctl::config::SubscriptionMode::Direct,
            )
            .map_err(|_| "订阅发布后权限检查失败".to_owned())?,
            Err(error) => {
                if let Some(previous) = previous {
                    store
                        .write_relative_locked("etc/sbctl/protocol-presets.json", &previous)
                        .map_err(|_| "组合源文件恢复失败")?;
                } else {
                    std::fs::remove_file(&path).map_err(|_| "组合源文件恢复失败")?;
                }
                return Err(error.to_string());
            }
        }
        println!(
            "已将 {} 个组合加入现有订阅；订阅域名、路径及凭据保持不变",
            nodes.len()
        );
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("组合发布失败：{error}");
            ExitCode::from(2)
        }
    }
}
