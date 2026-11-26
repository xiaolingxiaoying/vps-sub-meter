//! Pure dual-core server and multi-format client subscription compiler.

pub mod client;
pub mod server;

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::secrets::Secrets;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("配置校验失败: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("读取模板文件失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("模板格式错误: {0}")]
    Format(String),
    #[error("无法为 IP 模式固定 TLS 证书 {path}: {reason}")]
    TlsCertificate { path: String, reason: String },
}

#[derive(Clone, Debug)]
pub struct ClientBundle {
    pub clash_yaml: String,
    pub sing_box_json: Option<String>,
    pub shadowrocket_txt: String,
    pub uri_txt: String,
    pub direct_node_names: Vec<String>,
    pub singbox_node_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledDeployment {
    pub xray_server: Option<String>,
    pub singbox_server: Option<String>,
    pub clients: ClientBundle,
    pub argo_clients: Option<ClientBundle>,
}

#[derive(Clone, Debug)]
pub struct CertPaths {
    pub fullchain: String,
    pub privkey: String,
}

pub fn cert_paths(root: &Path, config: &Config) -> CertPaths {
    let base = if root == Path::new("/") {
        PathBuf::from("/var/lib/vps-sub-meter/certificates")
    } else {
        root.join("var/lib/vps-sub-meter/certificates")
    };
    let dir = base.join(config.tls_sni());
    CertPaths {
        fullchain: dir.join("fullchain.pem").to_string_lossy().into_owned(),
        privkey: dir.join("privkey.pem").to_string_lossy().into_owned(),
    }
}

pub fn compile_all(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
) -> Result<CompiledDeployment, CompileError> {
    config.validate()?;
    let certs = cert_paths(root, config);
    let xray_server = server::compile_xray_server(config, secrets, &certs)?;
    let singbox_server = server::compile_singbox_server(config, secrets, &certs)?;
    let clients = client::compile_direct_clients(root, config, secrets)?;
    let argo_clients = if config.argo.enabled {
        Some(client::compile_argo_clients(root, config, secrets)?)
    } else {
        None
    };
    Ok(CompiledDeployment {
        xray_server,
        singbox_server,
        clients,
        argo_clients,
    })
}
