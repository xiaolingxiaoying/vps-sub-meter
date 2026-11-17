//! Explicit alternate sources. Defaults remain the upstream official releases.
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};
pub struct CustomSource<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
}
pub fn validate(source: &CustomSource<'_>) -> Result<(), String> {
    let url = url::Url::parse(source.url).map_err(|_| "下载链接无效")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || source.url.chars().any(char::is_control)
    {
        return Err("自定义下载仅支持无用户名/密码的 HTTPS 链接；不执行网页或安装脚本".into());
    }
    if source.sha256.len() != 64 || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(
            "自定义下载必须提供文件的 64 位 SHA-256；哈希只能证明文件一致，不能证明来源可信".into(),
        );
    }
    Ok(())
}
pub fn download(
    source: &CustomSource<'_>,
    output: &Path,
    core: crate::core::CoreKind,
    version: &str,
) -> Result<(), String> {
    validate(source)?;
    let temp = tempfile::tempdir().map_err(|_| "无法创建下载目录")?;
    let archive = temp.path().join("archive");
    // A direct URL may include a private query token. Keep it out of argv.
    let curl_config = temp.path().join("curl.conf");
    let escaped_url = source.url.replace('\\', "\\\\").replace('"', "\\\"");
    std::fs::write(&curl_config, format!("url = \"{escaped_url}\"\n"))
        .map_err(|_| "无法写入下载配置")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl_config, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护下载配置")?;
    }
    let result = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "180",
            "--max-filesize",
            "134217728",
            "--output",
        ])
        .arg(&archive)
        .arg("--config")
        .arg(&curl_config)
        .output()
        .map_err(|_| "需要 curl 才能下载")?;
    if !result.status.success() {
        return Err("自定义下载失败；请检查 HTTPS 链接和网络".into());
    }
    let mut file = std::fs::File::open(&archive).map_err(|_| "下载文件不可读")?;
    if file.metadata().map_err(|_| "下载文件不可读")?.len() > 134217728 {
        return Err("下载文件超过 128 MiB".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "下载文件不可读")?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != source.sha256.to_ascii_lowercase() {
        return Err("自定义下载 SHA-256 不匹配，旧内核未修改".into());
    }
    let mut prefix = [0; 4];
    std::fs::File::open(&archive)
        .map_err(|_| "文件不可读")?
        .read_exact(&mut prefix)
        .map_err(|_| "下载内容太短，不是内核")?;
    if prefix == *b"\x7fELF" {
        std::fs::copy(&archive, output).map_err(|_| "无法保存内核")?;
        return Ok(());
    }
    let mut command = if prefix[..2] == [0x1f, 0x8b] && core == crate::core::CoreKind::SingBox {
        let mut command = Command::new("tar");
        command.arg("-xzOf").arg(&archive).arg(format!(
            "sing-box-{version}-linux-{}/sing-box",
            crate::update::official_release_arch()
        ));
        command
    } else if prefix[..2] == *b"PK" && core == crate::core::CoreKind::Xray {
        let mut command = Command::new("unzip");
        command.arg("-p").arg(&archive).arg("xray");
        command
    } else {
        return Err("链接应直接下载 Linux 内核、sing-box 官方格式 tar.gz 或 Xray zip；不能是网页、deb 或脚本".into());
    };
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "解压需要 tar / unzip")?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(134217729)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取解压结果失败")?;
    if bytes.len() > 134217728 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("解压后的内核超过 128 MiB".into());
    }
    if !child.wait().map_err(|_| "解压失败")?.success() || !bytes.starts_with(b"\x7fELF") {
        return Err("归档不包含可用 Linux 内核".into());
    }
    std::fs::write(output, bytes).map_err(|_| "无法保存内核".into())
}
