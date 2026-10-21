//! Managed mutations on Unix stay relative to pinned directory descriptors.
//! A directory rename or a replacement symlink cannot redirect a later create,
//! commit, cleanup, or unlink to a different tree.

use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn validate_file_name(name: &OsStr) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || name.to_string_lossy().contains(['/', '\\'])
        || name.as_encoded_bytes().contains(&0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "managed name must be a single file name",
        ));
    }
    Ok(())
}

fn split_file(path: &Path) -> io::Result<(&Path, &OsStr)> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "managed file has no name"))?;
    validate_file_name(name)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "managed file has no parent"))?;
    Ok((parent, name))
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
        io::{AsRawFd, FromRawFd},
    };

    pub(super) fn c_name(name: &OsStr) -> io::Result<CString> {
        validate_file_name(name)?;
        CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "managed name contains NUL"))
    }

    pub(super) fn open_at(directory: &File, name: &CStr, flags: libc::c_int) -> io::Result<File> {
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags,
                0o600 as libc::mode_t,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(super) fn open_directory(path: &Path, create: bool) -> io::Result<File> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut current = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(if path.is_absolute() { "/" } else { "." })?;
        for component in path.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => c_name(name)?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "managed directory must not contain parent components",
                    ));
                }
            };
            current = match open_at(&current, &name, flags) {
                Ok(directory) => directory,
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    let created =
                        unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
                    if !created {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(error);
                        }
                    }
                    let directory = open_at(&current, &name, flags)?;
                    if created {
                        directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
                    }
                    directory
                }
                Err(error) => return Err(error),
            };
        }
        Ok(current)
    }

    pub(crate) fn create_directory(path: &Path) -> io::Result<()> {
        open_directory(path, true).map(|_| ())
    }

    struct StagedWrite {
        directory: File,
        temporary: CString,
        destination: CString,
        file: File,
        committed: bool,
    }

    impl StagedWrite {
        fn new(directory: &File, name: &OsStr) -> io::Result<Self> {
            let destination = c_name(name)?;
            let directory = directory.try_clone()?;
            let temporary = CString::new(format!(
                ".sbctl.{}.{}.tmp",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap();
            let file = open_at(
                &directory,
                &temporary,
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )?;
            Ok(Self {
                directory,
                temporary,
                destination,
                file,
                committed: false,
            })
        }

        fn commit(&mut self) -> io::Result<()> {
            self.file.sync_all()?;
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    self.temporary.as_ptr(),
                    self.directory.as_raw_fd(),
                    self.destination.as_ptr(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            self.committed = true;
            // Rename already committed visible bytes. A durability failure must
            // not be reported as though the old target were still in place.
            let _ = self.directory.sync_all();
            Ok(())
        }
    }

    impl Drop for StagedWrite {
        fn drop(&mut self) {
            if !self.committed {
                unsafe {
                    libc::unlinkat(self.directory.as_raw_fd(), self.temporary.as_ptr(), 0);
                }
            }
        }
    }

    pub(crate) fn atomic_write(
        path: &Path,
        contents: &[u8],
        before_commit: impl FnOnce(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        let (parent, name) = split_file(path)?;
        let directory = open_directory(parent, true)?;
        let mut staged = StagedWrite::new(&directory, name)?;
        staged.file.write_all(contents)?;
        before_commit(&staged.file)?;
        staged.commit()
    }

    pub(crate) fn open_lock(path: &Path) -> io::Result<File> {
        let directory = open_directory(path, true)?;
        open_lock_in(&directory)
    }

    pub(super) fn open_lock_in(directory: &File) -> io::Result<File> {
        let name = c_name(OsStr::new(".operation.lock"))?;
        let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        loop {
            let file = match open_at(directory, &name, flags) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match open_at(directory, &name, flags | libc::O_CREAT | libc::O_EXCL) {
                        Ok(file) => file,
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(error) => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            };
            if !file.metadata()?.is_file() {
                return Err(io::Error::other("private storage must be a regular file"));
            }
            return Ok(file);
        }
    }

    pub(crate) fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let (parent, name) = split_file(path.as_ref())?;
        let directory = open_directory(parent, false)?;
        let name = c_name(name)?;
        unlink_in(&directory, &name)
    }

    fn unlink_in(directory: &File, name: &CStr) -> io::Result<()> {
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;
        use std::os::unix::fs::symlink;

        #[test]
        fn commit_cleanup_and_unlink_stay_in_pinned_directory_after_parent_replacement() {
            for commit in [false, true] {
                let fixture = tempfile::tempdir().unwrap();
                let original = fixture.path().join("original");
                let moved = fixture.path().join("moved");
                let outside = fixture.path().join("outside");
                fs::create_dir(&original).unwrap();
                fs::create_dir(&outside).unwrap();
                fs::write(outside.join("artifact"), b"private sentinel").unwrap();
                let directory = open_directory(&original, false).unwrap();
                let mut staged = StagedWrite::new(&directory, OsStr::new("artifact")).unwrap();
                staged.file.write_all(b"new managed bytes").unwrap();
                fs::rename(&original, &moved).unwrap();
                symlink(&outside, &original).unwrap();
                if commit {
                    staged.commit().unwrap();
                }
                drop(staged);
                assert_eq!(
                    fs::read(outside.join("artifact")).unwrap(),
                    b"private sentinel"
                );
                assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
                if commit {
                    assert_eq!(
                        fs::read(moved.join("artifact")).unwrap(),
                        b"new managed bytes"
                    );
                    unlink_in(&directory, &c_name(OsStr::new("artifact")).unwrap()).unwrap();
                }
                assert_eq!(
                    fs::read_dir(&moved).unwrap().count(),
                    0,
                    "no staging file may leak"
                );
            }
        }

        #[test]
        fn writes_replace_leaf_links_without_following_and_failed_commits_clean_up() {
            let fixture = tempfile::tempdir().unwrap();
            let private = fixture.path().join("private");
            let destination = fixture.path().join("destination");
            fs::write(&private, b"private sentinel").unwrap();
            symlink(&private, &destination).unwrap();
            atomic_write(&destination, b"new bytes", |_| Ok(())).unwrap();
            assert_eq!(fs::read(&private).unwrap(), b"private sentinel");
            assert_eq!(fs::read(&destination).unwrap(), b"new bytes");
            let blocked = fixture.path().join("blocked");
            fs::create_dir(&blocked).unwrap();
            assert!(atomic_write(&blocked, b"must fail", |_| Ok(())).is_err());
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
        }

        #[test]
        fn deletion_rejects_linked_parents_and_does_not_follow_leaf_links() {
            let fixture = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let victim = outside.path().join("artifact");
            fs::write(&victim, b"sentinel").unwrap();
            symlink(outside.path(), fixture.path().join("parent")).unwrap();
            assert!(remove_file(fixture.path().join("parent/artifact")).is_err());
            let leaf = fixture.path().join("leaf");
            symlink(&victim, &leaf).unwrap();
            remove_file(&leaf).unwrap();
            assert_eq!(fs::read(&victim).unwrap(), b"sentinel");
        }

        #[test]
        fn permissions_are_ready_at_commit_and_policy_failure_keeps_old_file() {
            let fixture = tempfile::tempdir().unwrap();
            let destination = fixture.path().join("artifact");
            fs::write(&destination, b"old bytes").unwrap();
            let result = atomic_write(&destination, b"must not publish", |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected policy failure",
                ))
            });
            assert!(result.is_err());
            assert_eq!(fs::read(&destination).unwrap(), b"old bytes");
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
            atomic_write(&destination, b"new bytes", |file| {
                file.set_permissions(fs::Permissions::from_mode(0o640))
            })
            .unwrap();
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o640
            );
            let directory = fixture.path().join("existing");
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o1771)).unwrap();
            open_lock(&directory).unwrap();
            atomic_write(&directory.join("nested/state"), b"state", |_| Ok(())).unwrap();
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o7777,
                0o1771
            );
            assert_eq!(
                fs::metadata(directory.join("nested"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(directory.join("nested/state"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;

    fn checked_directory(path: &Path, create: bool) -> io::Result<()> {
        let mut current = std::path::PathBuf::new();
        for component in path.components() {
            if matches!(component, Component::ParentDir) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "managed directory must not contain parent components",
                ));
            }
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0 {
                            return Err(io::Error::other(
                                "managed directory must not be a reparse point",
                            ));
                        }
                    }
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(io::Error::other(
                            "managed directory must be a regular directory",
                        ));
                    }
                }
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current)?
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn create_directory(path: &Path) -> io::Result<()> {
        checked_directory(path, true)
    }
    pub(crate) fn atomic_write(
        path: &Path,
        contents: &[u8],
        before_commit: impl FnOnce(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        let (parent, _) = split_file(path)?;
        checked_directory(parent, true)?;
        let temporary = parent.join(format!(
            ".sbctl.{}.{}.tmp",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(contents)?;
            before_commit(&file)?;
            file.sync_all()?;
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    pub(crate) fn open_lock(directory: &Path) -> io::Result<File> {
        checked_directory(directory, true)?;
        let path = directory.join(".operation.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::other("private storage must not be a symlink"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("private storage must be a regular file"));
        }
        Ok(file)
    }
    pub(crate) fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let (parent, _) = split_file(path)?;
        checked_directory(parent, false)?;
        fs::remove_file(path)
    }
}

pub(crate) use platform::{atomic_write, create_directory, open_lock, remove_file};

#[path = "managed_directory.rs"]
mod directory;
pub(crate) use directory::{read_pair, replace_files};
