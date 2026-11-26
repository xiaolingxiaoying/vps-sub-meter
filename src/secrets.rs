//! Root-private cryptographic secrets (`/etc/vps-sub-meter/secrets.json`, 0600).
//! Kept separate from `config.toml` and never placed into client generation dirs.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

use crate::config::Config;
use crate::fs_util;

pub const SECRETS_REL: &str = "etc/vps-sub-meter/secrets.json";

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("密钥文件读写失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("密钥文件解析失败: {0}")]
    Parse(String),
    #[error("生成密钥失败: {0}")]
    Keygen(String),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionToken {
    pub name: String,
    pub token: String,
    pub created_at: i64,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VlessRealitySecrets {
    pub uuid: String,
    pub x25519_private_key: String,
    pub x25519_public_key: String,
    pub short_id: String,
    pub mldsa65_seed: String,
    pub mldsa65_verify: String,
    pub vless_decryption: String,
    pub vless_encryption: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VlessXhttpSecrets {
    pub uuid: String,
    pub path: String,
    pub vless_decryption: String,
    pub vless_encryption: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AnytlsSecrets {
    pub password: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Hysteria2Secrets {
    pub password: String,
    pub obfs_password: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TuicSecrets {
    pub uuid: String,
    pub password: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArgoWsSecret {
    pub uuid: String,
    pub path: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Secrets {
    pub tokens: Vec<SubscriptionToken>,
    pub vless_reality_pqc: VlessRealitySecrets,
    pub vless_xhttp_pqc: VlessXhttpSecrets,
    pub anytls_tcp: AnytlsSecrets,
    pub hysteria2_hop: Hysteria2Secrets,
    pub tuic_bbr: TuicSecrets,
    pub argo_ws: BTreeMap<String, ArgoWsSecret>,
}

impl Secrets {
    pub fn path(root: &Path) -> PathBuf {
        root.join(SECRETS_REL)
    }

    pub fn load_or_default(root: &Path) -> Result<Self, SecretsError> {
        match fs_util::read_no_follow(&Self::path(root)) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| SecretsError::Parse(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<(), SecretsError> {
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| SecretsError::Parse(e.to_string()))?;
        fs_util::write_atomic(&Self::path(root), &bytes, 0o600)?;
        Ok(())
    }

    pub fn primary_token(&self) -> Option<&str> {
        self.tokens.first().map(|t| t.token.as_str())
    }

    /// Populate any missing secrets. Calls official `xray` CLI for ML-DSA-65
    /// and ML-KEM-768 pairs when `xray_bin` is provided.
    pub fn ensure_populated(
        &mut self,
        config: &Config,
        xray_bin: Option<&Path>,
    ) -> Result<(), SecretsError> {
        if self.tokens.is_empty() {
            self.tokens.push(SubscriptionToken {
                name: "default".into(),
                token: random_urlsafe(32)?,
                created_at: chrono::Utc::now().timestamp(),
            });
        }
        if self.vless_reality_pqc.uuid.is_empty() {
            self.vless_reality_pqc.uuid = random_uuid()?;
        }
        if self.vless_reality_pqc.x25519_private_key.is_empty()
            || self.vless_reality_pqc.x25519_public_key.is_empty()
        {
            let (priv_k, pub_k) = generate_x25519()?;
            self.vless_reality_pqc.x25519_private_key = priv_k;
            self.vless_reality_pqc.x25519_public_key = pub_k;
        }
        if self.vless_reality_pqc.short_id.is_empty() {
            self.vless_reality_pqc.short_id = random_hex(8)?;
        }
        if config.nodes.vless_reality_pqc.enabled
            && (self.vless_reality_pqc.mldsa65_seed.is_empty()
                || self.vless_reality_pqc.mldsa65_verify.is_empty())
        {
            let xray = xray_bin.ok_or_else(|| {
                SecretsError::Keygen("缺少 xray 二进制，无法生成 ML-DSA-65 抗量子密钥".into())
            })?;
            let (seed, verify) = run_xray_mldsa65(xray)?;
            self.vless_reality_pqc.mldsa65_seed = seed;
            self.vless_reality_pqc.mldsa65_verify = verify;
        }
        if config.nodes.vless_reality_pqc.enabled
            && (self.vless_reality_pqc.vless_decryption.is_empty()
                || self.vless_reality_pqc.vless_encryption.is_empty())
        {
            let xray = xray_bin.ok_or_else(|| {
                SecretsError::Keygen("缺少 xray 二进制，无法生成 VLESS ML-KEM-768 密钥".into())
            })?;
            let (dec, enc) = run_xray_vlessenc(xray)?;
            self.vless_reality_pqc.vless_decryption = dec;
            self.vless_reality_pqc.vless_encryption = enc;
        }

        if self.vless_xhttp_pqc.uuid.is_empty() {
            self.vless_xhttp_pqc.uuid = random_uuid()?;
        }
        if self.vless_xhttp_pqc.path.is_empty() {
            self.vless_xhttp_pqc.path = format!("/{}", random_urlsafe(16)?);
        }
        if config.nodes.vless_xhttp_pqc.enabled
            && (self.vless_xhttp_pqc.vless_decryption.is_empty()
                || self.vless_xhttp_pqc.vless_encryption.is_empty())
        {
            let xray = xray_bin.ok_or_else(|| {
                SecretsError::Keygen(
                    "缺少 xray 二进制，无法生成 XHTTP VLESS Encryption 密钥".into(),
                )
            })?;
            let (dec, enc) = run_xray_vlessenc(xray)?;
            self.vless_xhttp_pqc.vless_decryption = dec;
            self.vless_xhttp_pqc.vless_encryption = enc;
        }

        if self.anytls_tcp.password.is_empty() {
            self.anytls_tcp.password = random_urlsafe(32)?;
        }
        if self.hysteria2_hop.password.is_empty() {
            self.hysteria2_hop.password = random_urlsafe(32)?;
        }
        if self.hysteria2_hop.obfs_password.is_empty() {
            self.hysteria2_hop.obfs_password = random_urlsafe(24)?;
        }
        if self.tuic_bbr.uuid.is_empty() {
            self.tuic_bbr.uuid = random_uuid()?;
        }
        if self.tuic_bbr.password.is_empty() {
            self.tuic_bbr.password = random_urlsafe(32)?;
        }

        for ws in &config.argo.ws_inbounds {
            let entry = self.argo_ws.entry(ws.id.clone()).or_default();
            if entry.uuid.is_empty() {
                entry.uuid = random_uuid()?;
            }
            if entry.path.is_empty() {
                entry.path = format!("/{}", random_urlsafe(16)?);
            }
        }
        Ok(())
    }
}

pub fn random_urlsafe(byte_len: usize) -> Result<String, SecretsError> {
    let mut buf = vec![0u8; byte_len];
    getrandom::fill(&mut buf).map_err(|_| SecretsError::Keygen("系统随机数不可用".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub fn random_hex(byte_len: usize) -> Result<String, SecretsError> {
    let mut buf = vec![0u8; byte_len];
    getrandom::fill(&mut buf).map_err(|_| SecretsError::Keygen("系统随机数不可用".into()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn random_uuid() -> Result<String, SecretsError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| SecretsError::Keygen("系统随机数不可用".into()))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

pub fn generate_x25519() -> Result<(String, String), SecretsError> {
    let mut priv_bytes = [0u8; 32];
    getrandom::fill(&mut priv_bytes)
        .map_err(|_| SecretsError::Keygen("系统随机数不可用".into()))?;
    // Clamp scalar for standard X25519 compatibility with Xray output.
    priv_bytes[0] &= 248;
    priv_bytes[31] &= 127;
    priv_bytes[31] |= 64;
    let pub_bytes = x25519(priv_bytes, X25519_BASEPOINT_BYTES);
    Ok((
        URL_SAFE_NO_PAD.encode(priv_bytes),
        URL_SAFE_NO_PAD.encode(pub_bytes),
    ))
}

fn run_xray_mldsa65(xray_bin: &Path) -> Result<(String, String), SecretsError> {
    let out = Command::new(xray_bin).arg("mldsa65").output()?;
    if !out.status.success() {
        return Err(SecretsError::Keygen("xray mldsa65 执行失败".into()));
    }
    parse_mldsa65_output(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_mldsa65_output(stdout: &str) -> Result<(String, String), SecretsError> {
    let mut seed = None;
    let mut verify = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Seed:") {
            seed = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("Verify:") {
            verify = Some(rest.trim().to_owned());
        }
    }
    match (seed, verify) {
        (Some(s), Some(v)) if !s.is_empty() && !v.is_empty() => Ok((s, v)),
        _ => Err(SecretsError::Keygen("无法解析 xray mldsa65 输出".into())),
    }
}

fn run_xray_vlessenc(xray_bin: &Path) -> Result<(String, String), SecretsError> {
    let out = Command::new(xray_bin).arg("vlessenc").output()?;
    if !out.status.success() {
        return Err(SecretsError::Keygen("xray vlessenc 执行失败".into()));
    }
    parse_vlessenc_mlkem_output(&String::from_utf8_lossy(&out.stdout))
}

/// Picks the Post-Quantum `ML-KEM-768` section from `xray vlessenc` output.
pub fn parse_vlessenc_mlkem_output(stdout: &str) -> Result<(String, String), SecretsError> {
    let mut in_mlkem = false;
    let mut decryption = None;
    let mut encryption = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Authentication:") {
            in_mlkem = trimmed.contains("ML-KEM-768");
            continue;
        }
        if !in_mlkem {
            continue;
        }
        if let Some(val) = extract_quoted_field(trimmed, "\"decryption\"") {
            decryption = Some(val);
        } else if let Some(val) = extract_quoted_field(trimmed, "\"encryption\"") {
            encryption = Some(val);
        }
    }
    match (decryption, encryption) {
        (Some(d), Some(e)) if !d.is_empty() && !e.is_empty() => Ok((d, e)),
        _ => Err(SecretsError::Keygen(
            "无法解析 xray vlessenc ML-KEM-768 输出".into(),
        )),
    }
}

fn extract_quoted_field(line: &str, key: &str) -> Option<String> {
    let (_, after_key) = line.split_once(key)?;
    let (_, after_colon) = after_key.split_once(':')?;
    let trimmed = after_colon.trim().trim_end_matches(',');
    serde_json::from_str::<String>(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_xray_mldsa65_and_vlessenc_outputs() {
        let mldsa = "Seed: ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefg\nVerify: VERIFYKEYDATA\n";
        let (seed, verify) = parse_mldsa65_output(mldsa).unwrap();
        assert_eq!(seed, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefg");
        assert_eq!(verify, "VERIFYKEYDATA");

        let vlessenc = r#"
Authentication: X25519, not Post-Quantum
"decryption": "mlkem768x25519plus.native.600s.X25519_DEC"
"encryption": "mlkem768x25519plus.native.0rtt.X25519_ENC"

Authentication: ML-KEM-768, Post-Quantum
"decryption": "mlkem768x25519plus.native.600s.PQ_DEC_KEY"
"encryption": "mlkem768x25519plus.native.0rtt.PQ_ENC_KEY"
"#;
        let (dec, enc) = parse_vlessenc_mlkem_output(vlessenc).unwrap();
        assert_eq!(dec, "mlkem768x25519plus.native.600s.PQ_DEC_KEY");
        assert_eq!(enc, "mlkem768x25519plus.native.0rtt.PQ_ENC_KEY");
    }

    #[test]
    fn partial_empty_secrets_deserialize_with_defaults() {
        let s1: Secrets = serde_json::from_str(r#"{"tokens":[]}"#).unwrap();
        assert!(s1.tokens.is_empty());
        let s2: Secrets = serde_json::from_str(r#"{"vless_reality_pqc":{}}"#).unwrap();
        assert!(s2.vless_reality_pqc.uuid.is_empty());
    }
}
