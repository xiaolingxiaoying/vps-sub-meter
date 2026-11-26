//! Safe filesystem primitives: symlink-free reads and atomic mode-pinned writes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

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
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "路径缺少父目录"))?;
    fs::create_dir_all(parent)?;
    if let Ok(meta) = fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "拒绝写入符号链接目标",
        ));
    }
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
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
