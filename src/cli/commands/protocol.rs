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

pub(crate) fn run(command: ProtocolCommand) -> ExitCode {
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
