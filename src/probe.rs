//! A short-lived client tests one node without changing managed services.
use crate::{
    canonical::CanonicalNode,
    config::{CertificateMode, DeploymentConfig},
    core::CoreKind,
    nodes::{Data, Error, Node, Result},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(serde::Serialize)]
pub struct Report {
    pub id: String,
    pub core: CoreKind,
    pub http_status: u16,
    pub time_seconds: f64,
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn node(
    root: &Path,
    config: &DeploymentConfig,
    node: &Node,
    url: &str,
    client_bin: Option<&Path>,
) -> Result<Report> {
    if !node.enabled {
        return Err(Error("节点已停用，无法测试".into()));
    }
    let target = url::Url::parse(url).map_err(|_| Error("测速 URL 无效".into()))?;
    if !matches!(target.scheme(), "http" | "https")
        || target.host_str().is_none()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Err(Error(
            "测速 URL 必须为不含凭据或 fragment 的 HTTP(S) 地址".into(),
        ));
    }
    let xhttp = matches!(&node.data,Data::Combination(n)if n.preset.is_xhttp());
    let mut core = if xhttp
        || (node.core == CoreKind::Xray
            && matches!(node.data, Data::Legacy(CanonicalNode::VlessReality { .. })))
        || !root
            .join(crate::deployment::binary(CoreKind::SingBox))
            .is_file()
    {
        CoreKind::Xray
    } else {
        CoreKind::SingBox
    };
    if let Some(binary) = client_bin {
        let output = Command::new(binary)
            .arg("version")
            .output()
            .map_err(|_| Error("无法识别客户端检查程序".into()))?;
        let text = String::from_utf8_lossy(&output.stdout);
        core = if text.starts_with("sing-box version ") {
            CoreKind::SingBox
        } else if text.starts_with("Xray ") {
            CoreKind::Xray
        } else {
            return Err(Error("客户端检查程序身份不受支持".into()));
        };
        if xhttp && core == CoreKind::SingBox {
            return Err(Error("该 XHTTP 节点需要 Xray 检查客户端".into()));
        }
    }
    let binary = client_bin
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(crate::deployment::binary(core)));
    if !binary.is_file() {
        return Err(Error(format!(
            "缺少 {core} 客户端检查程序；安装该内核不会自动启用其服务"
        )));
    }
    let mut outbound = if core == CoreKind::SingBox {
        crate::subscription::managed::client_node(
            config,
            node,
            crate::subscription::SubscriptionFormat::SingBox,
        )
        .map_err(|_| Error("该节点没有可用的 sing-box 客户端表示".into()))?
    } else {
        match &node.data {
            Data::Combination(n) => n
                .xray_outbound()
                .map_err(|_| Error("Xray 客户端配置生成失败".into()))?,
            Data::Legacy(CanonicalNode::VlessReality {
                host,
                port,
                uuid,
                public_key,
                short_id,
                decoy_sni,
                ..
            }) => {
                json!({"protocol":"vless","settings":{"vnext":[{"address":host,"port":port,"users":[{"id":uuid,"encryption":"none","flow":"xtls-rprx-vision"}]}]},"streamSettings":{"network":"raw","security":"reality","realitySettings":{"serverName":decoy_sni,"fingerprint":"chrome","password":public_key,"shortId":short_id}}})
            }
            Data::Legacy(CanonicalNode::VmessWebsocket {
                host,
                port,
                uuid,
                path,
                tls_server_name,
            }) => {
                json!({"protocol":"vmess","settings":{"vnext":[{"address":host,"port":port,"users":[{"id":uuid,"alterId":0,"security":"auto"}]}]},"streamSettings":{"network":"ws","security":"tls","wsSettings":{"path":path},"tlsSettings":{"serverName":tls_server_name}}})
            }
            _ => return Err(Error("该节点需要安装 sing-box 客户端检查程序".into())),
        }
    };
    if core == CoreKind::Xray
        && config.certificate_mode == CertificateMode::SelfSigned
        && outbound["streamSettings"]["security"] == "tls"
    {
        let sni = config.protocol_server_name();
        let bytes = crate::config::read_managed_file_no_follow(
            &root.join(format!("var/lib/sbctl/certificates/{sni}/cert.pem")),
        )
        .map_err(|_| Error("无法读取托管自签名证书以验证连接".into()))?;
        let cert = rustls_pemfile::certs(&mut std::io::Cursor::new(bytes))
            .next()
            .ok_or_else(|| Error("证书没有 PEM 数据".into()))?
            .map_err(|_| Error("证书 PEM 无效".into()))?;
        outbound["streamSettings"]["tlsSettings"]["pinnedPeerCertSha256"] =
            json!(format!("{:x}", Sha256::digest(cert.as_ref())));
    }
    outbound["tag"] = json!("probe");
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|_| Error("无法分配本机检查端口".into()))?;
    let port = reservation
        .local_addr()
        .map_err(|_| Error("无法读取本机端口".into()))?
        .port();
    let document = if core == CoreKind::SingBox {
        json!({"log":{"level":"error"},"inbounds":[{"type":"socks","listen":"127.0.0.1","listen_port":port}],"outbounds":[outbound],"route":{"final":"probe"}})
    } else {
        json!({"log":{"loglevel":"error"},"inbounds":[{"protocol":"socks","listen":"127.0.0.1","port":port,"settings":{"auth":"noauth"}}],"outbounds":[outbound]})
    };
    let mut file = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .map_err(|_| Error("无法准备私有检查配置".into()))?;
    file.write_all(document.to_string().as_bytes())
        .map_err(|_| Error("无法写入检查配置".into()))?;
    file.flush().map_err(|_| Error("无法刷新检查配置".into()))?;
    let path = file.into_temp_path();
    core.check_config(&binary, &document.to_string())
        .map_err(|_| Error("客户端检查配置未通过内核校验".into()))?;
    drop(reservation);
    let mut child = Child(
        Command::new(&binary)
            .args(core.run_arguments())
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error("检查客户端无法启动".into()))?,
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if child
            .0
            .try_wait()
            .map_err(|_| Error("无法检查客户端状态".into()))?
            .is_some()
        {
            return Err(Error("检查客户端启动后退出".into()));
        }
        if std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok() {
            break;
        }
        if Instant::now() > deadline {
            return Err(Error("检查客户端启动超时".into()));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let sink = tempfile::NamedTempFile::new()
        .map_err(|_| Error("无法创建检查输出文件".into()))?
        .into_temp_path();
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "15",
            "--proxy",
        ])
        .arg(format!("socks5h://127.0.0.1:{port}"))
        .arg("--output")
        .arg(&sink)
        .args(["--write-out", "%{http_code} %{time_total}", "--url", url])
        .output()
        .map_err(|_| Error("检查需要 curl".into()))?;
    if !output.status.success() {
        return Err(Error("通过该节点的真实请求失败或超时（未修改服务）".into()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.split_whitespace();
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error("检查响应无效".into()))?;
    let seconds: f64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error("检查耗时无效".into()))?;
    if !(200..400).contains(&status) {
        return Err(Error(format!("节点连接成功，但测试目标返回 HTTP {status}")));
    }
    Ok(Report {
        id: node.id.clone(),
        core,
        http_status: status,
        time_seconds: seconds,
    })
}
