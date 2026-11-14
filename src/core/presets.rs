//! Validated protocol combinations. Transport/security are fixed by the preset,
//! so arbitrary incompatible combinations cannot enter a generated artifact.
use super::xray::CertificatePaths;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    VlessXhttpReality,
    VlessXhttpTls,
    VlessWsTls,
    TrojanTcpTls,
}

impl Preset {
    pub fn is_xhttp(self) -> bool {
        matches!(self, Self::VlessXhttpReality | Self::VlessXhttpTls)
    }
    pub fn is_reality(self) -> bool {
        self == Self::VlessXhttpReality
    }
    pub fn is_trojan(self) -> bool {
        self == Self::TrojanTcpTls
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reality {
    pub private_key: String,
    pub public_key: String,
    pub short_id: String,
    pub target: String,
}

/// Debug intentionally omitted: this document contains proxy credentials.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub preset: Preset,
    pub server: String,
    pub port: u16,
    pub server_name: String,
    /// UUID for VLESS, independent password for Trojan.
    pub credential: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reality: Option<Reality>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid protocol preset: {0}")]
pub struct Error(pub &'static str);

impl Node {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() || self.name.chars().any(char::is_control) {
            return Err(Error(
                "name must be nonempty and contain no control characters",
            ));
        }
        if self.server.is_empty()
            || self
                .server
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '@' | '[' | ']'))
        {
            return Err(Error("server must be a bare hostname or IP address"));
        }
        if self.server.contains(':') && self.server.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(Error("invalid IPv6 server"));
        }
        if self.server.parse::<std::net::IpAddr>().is_err()
            && (self.server.len() > 253
                || !self
                    .server
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')))
        {
            return Err(Error("server must be an ASCII hostname or IP address"));
        }
        if self.port < 10000 {
            return Err(Error("listener port must be in 10000-65535"));
        }
        if self.server_name.is_empty()
            || !self
                .server_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        {
            return Err(Error("server_name must be a hostname"));
        }
        if self.preset.is_trojan() {
            if self.credential.is_empty() || self.credential.chars().any(char::is_control) {
                return Err(Error("Trojan password is empty or invalid"));
            }
            if self.path.is_some() {
                return Err(Error("TCP Trojan does not accept a transport path"));
            }
        } else {
            let parts: Vec<_> = self.credential.split('-').collect();
            if parts.iter().map(|p| p.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12]
                || !parts
                    .iter()
                    .all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(Error("VLESS credential must be a UUID"));
            }
            if self
                .path
                .as_deref()
                .is_none_or(|p| !p.starts_with('/') || p.chars().any(char::is_control))
            {
                return Err(Error(
                    "transport path must start with / and contain no control characters",
                ));
            }
        }
        match (&self.reality, self.preset.is_reality()) {
            (Some(reality), true) => {
                let decode = |value: &str| {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(value)
                        .ok()
                        .and_then(|v| <[u8; 32]>::try_from(v).ok())
                };
                let private =
                    decode(&reality.private_key).ok_or(Error("invalid Reality private key"))?;
                let public =
                    decode(&reality.public_key).ok_or(Error("invalid Reality public key"))?;
                if x25519_dalek::x25519(private, x25519_dalek::X25519_BASEPOINT_BYTES) != public {
                    return Err(Error("Reality key pair does not match"));
                }
                if reality.short_id.is_empty()
                    || reality.short_id.len() > 16
                    || !reality.short_id.len().is_multiple_of(2)
                    || !reality.short_id.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error(
                        "Reality short_id must be even-length hexadecimal, at most 16 characters",
                    ));
                }
                let target = url::Url::parse(&format!("tcp://{}", reality.target))
                    .map_err(|_| Error("invalid Reality target"))?;
                if target.host_str().is_none()
                    || target.port().is_none()
                    || !matches!(target.path(), "" | "/")
                    || target.query().is_some()
                    || target.fragment().is_some()
                    || !target.username().is_empty()
                    || target.password().is_some()
                {
                    return Err(Error("Reality target must contain only host:port"));
                }
            }
            (None, false) => (),
            _ => return Err(Error("Reality settings must match the selected preset")),
        }
        Ok(())
    }

    fn xray_stream(
        &self,
        server: bool,
        certificate: Option<&CertificatePaths<'_>>,
    ) -> Result<Value, Error> {
        let mut stream = json!({"network": if self.preset.is_xhttp() { "xhttp" } else if self.preset.is_trojan() { "raw" } else { "ws" },
            "security": if self.preset.is_reality() { "reality" } else { "tls" }});
        if self.preset.is_xhttp() {
            stream["xhttpSettings"] = json!({"path": self.path, "mode": "auto"});
        } else if !self.preset.is_trojan() {
            stream["wsSettings"] = json!({"path": self.path});
        }
        if let Some(reality) = &self.reality {
            stream["realitySettings"] = if server {
                json!({"target": reality.target, "serverNames": [self.server_name], "privateKey": reality.private_key, "shortIds": [reality.short_id]})
            } else {
                json!({"serverName": self.server_name, "fingerprint": "chrome", "password": reality.public_key, "shortId": reality.short_id})
            };
        } else if server {
            let paths = certificate
                .filter(|p| !p.certificate.is_empty() && !p.private_key.is_empty())
                .ok_or(Error("TLS server requires certificate paths"))?;
            stream["tlsSettings"] = json!({"certificates": [{"certificateFile": paths.certificate, "keyFile": paths.private_key}]});
        } else {
            stream["tlsSettings"] = json!({"serverName": self.server_name});
        }
        Ok(stream)
    }

    pub fn xray_inbound(&self, certificate: Option<&CertificatePaths<'_>>) -> Result<Value, Error> {
        self.validate()?;
        let settings = if self.preset.is_trojan() {
            json!({"clients": [{"password": self.credential}]})
        } else {
            json!({"clients": [{"id": self.credential}], "decryption": "none"})
        };
        Ok(
            json!({"tag": self.name, "listen": "::", "port": self.port, "protocol": if self.preset.is_trojan() { "trojan" } else { "vless" },
            "settings": settings, "streamSettings": self.xray_stream(true, certificate)?}),
        )
    }

    pub fn xray_outbound(&self) -> Result<Value, Error> {
        self.validate()?;
        let settings = if self.preset.is_trojan() {
            json!({"servers": [{"address": self.server, "port": self.port, "password": self.credential}]})
        } else {
            json!({"vnext": [{"address": self.server, "port": self.port, "users": [{"id": self.credential, "encryption": "none"}]}]})
        };
        Ok(
            json!({"tag": self.name, "protocol": if self.preset.is_trojan() { "trojan" } else { "vless" }, "settings": settings, "streamSettings": self.xray_stream(false, None)?}),
        )
    }

    pub fn sing_box_outbound(&self) -> Result<Value, Error> {
        self.validate()?;
        if self.preset.is_xhttp() {
            return Err(Error(
                "XHTTP is unavailable in the supported official sing-box profiles",
            ));
        }
        let mut value = json!({"type": if self.preset.is_trojan() { "trojan" } else { "vless" }, "tag": self.name,
            "server": self.server, "server_port": self.port, "tls": {"enabled": true, "server_name": self.server_name}});
        value[if self.preset.is_trojan() {
            "password"
        } else {
            "uuid"
        }] = json!(self.credential);
        if !self.preset.is_trojan() {
            value["transport"] = json!({"type": "ws", "path": self.path});
        }
        Ok(value)
    }

    /// XHTTP export requires an explicitly compatible Mihomo target. Never
    /// infer support from a GUI application name or a generic Clash label.
    pub fn mihomo_proxy(&self, supports_xhttp: bool) -> Result<Value, Error> {
        self.validate()?;
        // Current server baseline is Xray 26.10.31. Mihomo explicitly does not
        // promise Reality interoperability with Xray 26.7.11 and later.
        if self.preset.is_reality() {
            return Err(Error(
                "Mihomo Reality interoperability with the Xray 26.10.31 server baseline is not validated",
            ));
        }
        if self.preset.is_xhttp() && !supports_xhttp {
            return Err(Error(
                "target Mihomo profile has no validated XHTTP support",
            ));
        }
        let mut value = json!({"name": self.name, "type": if self.preset.is_trojan() { "trojan" } else { "vless" }, "server": self.server, "port": self.port, "tls": true, "udp": true});
        value[if self.preset.is_trojan() {
            "password"
        } else {
            "uuid"
        }] = json!(self.credential);
        value[if self.preset.is_trojan() {
            "sni"
        } else {
            "servername"
        }] = json!(self.server_name);
        if self.preset.is_xhttp() {
            value["network"] = json!("xhttp");
            value["xhttp-opts"] = json!({"path": self.path, "mode": "auto"});
        } else if !self.preset.is_trojan() {
            value["network"] = json!("ws");
            value["ws-opts"] = json!({"path": self.path});
        }
        if let Some(reality) = &self.reality {
            value["reality-opts"] =
                json!({"public-key": reality.public_key, "short-id": reality.short_id});
            value["client-fingerprint"] = json!("chrome");
        }
        Ok(value)
    }

    pub fn share_uri(&self) -> Result<String, Error> {
        self.validate()?;
        let host = crate::canonical::uri_host(&self.server);
        let credential = url::form_urlencoded::byte_serialize(self.credential.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair(
                "security",
                if self.preset.is_reality() {
                    "reality"
                } else {
                    "tls"
                },
            )
            .append_pair("sni", &self.server_name)
            .append_pair(
                "type",
                if self.preset.is_xhttp() {
                    "xhttp"
                } else if self.preset.is_trojan() {
                    "tcp"
                } else {
                    "ws"
                },
            );
        if !self.preset.is_trojan() {
            query.append_pair("encryption", "none");
        }
        if let Some(path) = &self.path {
            query.append_pair("path", path);
        }
        if self.preset.is_xhttp() {
            query.append_pair("mode", "auto");
        }
        if let Some(reality) = &self.reality {
            query
                .append_pair("fp", "chrome")
                .append_pair("pbk", &reality.public_key)
                .append_pair("sid", &reality.short_id);
        }
        let name = url::form_urlencoded::byte_serialize(self.name.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        Ok(format!(
            "{}://{credential}@{host}:{}?{}#{name}",
            if self.preset.is_trojan() {
                "trojan"
            } else {
                "vless"
            },
            self.port,
            query.finish()
        ))
    }
}

/// Validate a complete set before rendering any result; duplicate listener
/// ports and names otherwise produce inconsistent subscriptions and services.
pub fn validate_nodes(nodes: &[Node]) -> Result<(), Error> {
    if nodes.is_empty() {
        return Err(Error("at least one node is required"));
    }
    let mut names = std::collections::HashSet::new();
    let mut ports = std::collections::HashSet::new();
    for node in nodes {
        node.validate()?;
        if !names.insert(&node.name) {
            return Err(Error("node names must be unique"));
        }
        if !ports.insert(node.port) {
            return Err(Error("listener ports must be unique"));
        }
    }
    Ok(())
}
