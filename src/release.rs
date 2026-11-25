//! Ed25519-signed release manifest verification and signing.

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const DEVELOPMENT_PUBLIC_KEY: [u8; 32] = [
    0x24, 0x7F, 0x88, 0xE1, 0x63, 0x24, 0x29, 0x86, 0xB7, 0x10, 0x7E, 0xB7, 0x04, 0xA9, 0x83, 0xE1,
    0x21, 0x86, 0xD2, 0x69, 0x7A, 0x39, 0x27, 0xB7, 0xE6, 0xB4, 0x2E, 0xC2, 0xB3, 0x64, 0x27, 0x2B,
];

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("未配置生产发布公钥 (VSM_RELEASE_PUBLIC_KEY_HEX)")]
    MissingTrustAnchor,
    #[error("发布公钥无效或仍为开发测试公钥")]
    InvalidTrustAnchor,
    #[error("读取发布清单失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("解析发布清单失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("不支持的清单 schema 版本 {0}")]
    Schema(u32),
    #[error("发布清单未签名或签名无效")]
    Signature,
    #[error("清单摘要或版本号格式无效: {0}")]
    Format(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema: u32,
    #[serde(alias = "sbctl")]
    pub vps_sub_meter: ReleaseArtifact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sing_box: Option<ReleaseArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xray: Option<ReleaseArtifact>,
    #[serde(skip_serializing, default)]
    pub signature: Option<String>,
}

fn parse_hex_32(hex: &str) -> Result<[u8; 32], ReleaseError> {
    let trimmed = hex.trim();
    if trimmed.len() != 64 {
        return Err(ReleaseError::InvalidTrustAnchor);
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&trimmed[i * 2..i * 2 + 2], 16)
            .map_err(|_| ReleaseError::InvalidTrustAnchor)?;
    }
    Ok(out)
}

fn trusted_public_key() -> Result<[u8; 32], ReleaseError> {
    if cfg!(any(test, feature = "test-signing")) {
        return Ok(DEVELOPMENT_PUBLIC_KEY);
    }
    let raw = option_env!("VSM_RELEASE_PUBLIC_KEY_HEX")
        .or(option_env!("SBCTL_RELEASE_PUBLIC_KEY_HEX"))
        .filter(|s| !s.trim().is_empty())
        .ok_or(ReleaseError::MissingTrustAnchor)?;
    let key = parse_hex_32(raw)?;
    if key == DEVELOPMENT_PUBLIC_KEY {
        return Err(ReleaseError::InvalidTrustAnchor);
    }
    VerifyingKey::from_bytes(&key).map_err(|_| ReleaseError::InvalidTrustAnchor)?;
    Ok(key)
}

pub fn validate_fields(manifest: &ReleaseManifest) -> Result<(), ReleaseError> {
    if manifest.schema != 1 {
        return Err(ReleaseError::Schema(manifest.schema));
    }
    validate_artifact("vps_sub_meter", &manifest.vps_sub_meter)?;
    if let Some(sb) = &manifest.sing_box {
        validate_artifact("sing_box", sb)?;
    }
    if let Some(xr) = &manifest.xray {
        validate_artifact("xray", xr)?;
    }
    Ok(())
}

fn validate_artifact(name: &str, art: &ReleaseArtifact) -> Result<(), ReleaseError> {
    if ["latest", "main", "master"].contains(&art.version.to_ascii_lowercase().as_str()) {
        return Err(ReleaseError::Format(format!("{name} 禁止浮动版本号")));
    }
    let parts: Vec<_> = art.version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) {
        return Err(ReleaseError::Format(format!("{name} 版本号非 x.y.z")));
    }
    if art.sha256.len() != 64 || !art.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ReleaseError::Format(format!("{name} SHA-256 无效")));
    }
    Ok(())
}

pub fn canonical_bytes(value: &serde_json::Value) -> Result<Vec<u8>, ReleaseError> {
    let mut out = Vec::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &serde_json::Value, out: &mut Vec<u8>) -> Result<(), ReleaseError> {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().filter(|k| k.as_str() != "signature").collect();
            keys.sort_unstable();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(&serde_json::to_vec(k)?);
                out.push(b':');
                write_canonical(&map[*k], out)?;
            }
            out.push(b'}');
        }
        serde_json::Value::Array(arr) => {
            out.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        other => out.extend_from_slice(&serde_json::to_vec(other)?),
    }
    Ok(())
}

pub fn verify_manifest_file(path: &Path) -> Result<ReleaseManifest, ReleaseError> {
    let bytes = fs::read(path)?;
    let raw_value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let manifest: ReleaseManifest = serde_json::from_value(raw_value.clone())?;
    validate_fields(&manifest)?;
    let sig_b64 = manifest
        .signature
        .as_deref()
        .ok_or(ReleaseError::Signature)?;
    let sig_bytes = STANDARD
        .decode(sig_b64)
        .map_err(|_| ReleaseError::Signature)?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| ReleaseError::Signature)?;
    let key =
        VerifyingKey::from_bytes(&trusted_public_key()?).map_err(|_| ReleaseError::Signature)?;
    let canonical = canonical_bytes(&raw_value)?;
    key.verify_strict(&canonical, &sig)
        .map_err(|_| ReleaseError::Signature)?;
    Ok(manifest)
}

pub fn sign_manifest_file(
    manifest_path: &Path,
    seed_path: &Path,
    output_path: &Path,
) -> Result<(), ReleaseError> {
    let bytes = fs::read(manifest_path)?;
    let mut raw_value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let manifest: ReleaseManifest = serde_json::from_value(raw_value.clone())?;
    validate_fields(&manifest)?;
    let seed = parse_hex_32(&fs::read_to_string(seed_path)?)?;
    let signing = SigningKey::from_bytes(&seed);
    let canonical = canonical_bytes(&raw_value)?;
    let sig = signing.sign(&canonical);
    raw_value
        .as_object_mut()
        .ok_or_else(|| ReleaseError::Format("manifest 必须是 JSON 对象".into()))?
        .insert(
            "signature".into(),
            serde_json::Value::String(STANDARD.encode(sig.to_bytes())),
        );
    fs::write(output_path, serde_json::to_vec_pretty(&raw_value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_verifies_release_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let seed_hex = "62ac3d5801b52a11da923ebbbccb88a67f154bde4339596f051ee2732e9a365f";
        let seed_path = dir.path().join("seed");
        fs::write(&seed_path, seed_hex).unwrap();
        let unsigned = dir.path().join("unsigned.json");
        let signed = dir.path().join("signed.json");
        fs::write(
            &unsigned,
            serde_json::json!({
                "schema": 1,
                "vps_sub_meter": {
                    "version": "2.0.0",
                    "sha256": "a".repeat(64),
                    "url": "https://example.com/vsm"
                }
            })
            .to_string(),
        )
        .unwrap();
        sign_manifest_file(&unsigned, &seed_path, &signed).unwrap();
        let verified = verify_manifest_file(&signed).unwrap();
        assert_eq!(verified.vps_sub_meter.version, "2.0.0");
    }
}
