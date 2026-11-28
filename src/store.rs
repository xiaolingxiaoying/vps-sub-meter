//! Unified filesystem primitives, runtime authorization, and generation storage contracts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use crate::config::VAR_DIR;

pub const CURRENT_REL: &str = "var/lib/vps-sub-meter/current.json";
pub const SERVE_AUTH_REL: &str = "var/lib/vps-sub-meter/serve-auth.json";
pub const CERT_RENEW_STATUS_REL: &str = "var/lib/vps-sub-meter/cert-renew-status.json";
pub const XRAY_CONFIG_REL: &str = "etc/vps-sub-meter/xray/config.json";
pub const SINGBOX_CONFIG_REL: &str = "etc/vps-sub-meter/sing-box/config.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertRenewStatus {
    pub domain: String,
    pub status: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub attempted_at: i64,
    #[serde(default)]
    pub last_success_at: Option<i64>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationMeta {
    pub id: String,
    pub files: Vec<String>,
    pub direct_nodes: Vec<String>,
    pub singbox_nodes: Vec<String>,
    #[serde(default)]
    pub argo_nodes: Vec<String>,
    #[serde(default)]
    pub argo_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_fingerprint: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ServeAuth {
    pub token_sha256: Vec<String>,
}

pub fn open_no_follow(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path)
}

pub fn read_no_follow(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_no_follow(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn read_to_string_no_follow(path: &Path) -> io::Result<String> {
    let bytes = read_no_follow(path)?;
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "非 UTF-8 文件"))
}

pub fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    write_atomic_inner(path, contents, mode, false)
}

pub fn write_atomic_preserving_owner(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    write_atomic_inner(path, contents, mode, true)
}

fn write_atomic_inner(
    path: &Path,
    contents: &[u8],
    mode: u32,
    preserve_owner: bool,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "路径缺少父目录"))?;
    fs::create_dir_all(parent)?;
    let previous = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "拒绝写入符号链接目标",
            ));
        }
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if preserve_owner && let Some(previous) = previous {
            let current = tmp.as_file().metadata()?;
            if (previous.uid(), previous.gid()) != (current.uid(), current.gid()) {
                std::os::unix::fs::fchown(
                    tmp.as_file(),
                    Some(previous.uid()),
                    Some(previous.gid()),
                )?;
            }
        }
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (mode, previous, preserve_owner);
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn lock_exclusive(dir: &Path) -> io::Result<File> {
    let file = open_lock(dir)?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

pub fn ensure_dir_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

pub fn open_lock(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    let lock_path = dir.join(".operation.lock");
    let mut opts = OpenOptions::new();
    opts.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.custom_flags(libc::O_NOFOLLOW);
        // Linux protected_regular rejects O_CREAT for another user's existing
        // lock in the sticky state directory, even when root opens it.
        let file = match opts.open(&lock_path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                match opts.clone().create_new(true).mode(0o600).open(&lock_path) {
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                        opts.open(&lock_path)?
                    }
                    result => result?,
                }
            }
            result => result?,
        };
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        opts.create(true).open(&lock_path)
    }
}

/// Sets owner/group by name without following symlinks; no-ops if account is absent (e.g. sandbox).
#[cfg(unix)]
pub fn chown_name(path: &Path, user: Option<&str>, group: Option<&str>) {
    use std::ffi::CString;

    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    let uid = match user {
        Some(name) => {
            let Ok(c) = CString::new(name) else { return };
            let pw = unsafe { libc::getpwnam(c.as_ptr()) };
            if pw.is_null() {
                return;
            }
            Some(unsafe { (*pw).pw_uid })
        }
        None => None,
    };
    let gid = match group {
        Some(name) => {
            let Ok(c) = CString::new(name) else { return };
            let gr = unsafe { libc::getgrnam(c.as_ptr()) };
            if gr.is_null() {
                return;
            }
            Some(unsafe { (*gr).gr_gid })
        }
        None => None,
    };
    let _ = std::os::unix::fs::lchown(path, uid, gid);
}

#[cfg(not(unix))]
pub fn chown_name(_path: &Path, _user: Option<&str>, _group: Option<&str>) {}

pub fn chown_tree(path: &Path, user: Option<&str>, group: Option<&str>) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    chown_name(path, user, group);
    if meta.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            chown_tree(&entry.path(), user, group);
        }
    }
}

pub fn current_generation(root: &Path) -> io::Result<Option<GenerationMeta>> {
    match read_no_follow(&root.join(CURRENT_REL)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("current.json 损坏: {e}"),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn read_generation_file(
    root: &Path,
    gen_id: &str,
    argo: bool,
    name: &str,
) -> io::Result<Option<String>> {
    if !gen_id.bytes().all(|b| b.is_ascii_hexdigit())
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Ok(None);
    }
    let mut path = root.join(VAR_DIR).join("generations").join(gen_id);
    if argo {
        path = path.join("argo");
    }
    path = path.join(name);
    match read_to_string_no_follow(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn read_cert_renew_status(root: &Path) -> Option<CertRenewStatus> {
    let bytes = read_no_follow(&root.join(CERT_RENEW_STATUS_REL)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn read_cert_renew_status_for_domain(root: &Path, domain: &str) -> Option<CertRenewStatus> {
    read_cert_renew_status(root).filter(|rec| rec.domain == domain)
}

pub fn read_serve_auth(root: &Path) -> io::Result<ServeAuth> {
    let bytes = read_no_follow(&root.join(SERVE_AUTH_REL))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("serve-auth.json 损坏: {e}"),
        )
    })
}

pub fn write_serve_auth(root: &Path, auth: &ServeAuth) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(auth)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_atomic_preserving_owner(&root.join(SERVE_AUTH_REL), &bytes, 0o640)
}

pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn config_revision(root: &Path) -> io::Result<String> {
    match read_no_follow(&root.join(crate::config::CONFIG_REL)) {
        Ok(bytes) => Ok(format!("{:x}", Sha256::digest(bytes))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

pub fn constant_time_hash_match(hashes: &[String], candidate_token: &str) -> bool {
    if candidate_token.is_empty() || candidate_token.len() > 128 {
        return false;
    }
    let cand_hex = hash_token(candidate_token);
    let cand_bytes = cand_hex.as_bytes();
    let mut matched = 0u8;
    for expected in hashes {
        if expected.len() != cand_bytes.len() {
            continue;
        }
        let mut diff = 0u8;
        for (a, b) in expected.as_bytes().iter().zip(cand_bytes.iter()) {
            diff |= a ^ b;
        }
        if diff == 0 {
            matched = 1;
        }
    }
    matched == 1
}
