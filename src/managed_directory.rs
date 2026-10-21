//! Publish a flat certificate directory as one generation. Linux production
//! uses a single directory exchange after all bytes and metadata are synced.
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const MAX_ENTRIES: usize = 1024;
const MAX_MATERIAL_BYTES: u64 = 1024 * 1024;

fn read_material(file: File) -> io::Result<Vec<u8>> {
    if !file.metadata()?.is_file() {
        return Err(io::Error::other(
            "certificate material must be a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MATERIAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MATERIAL_BYTES {
        return Err(io::Error::other("certificate material is too large"));
    }
    Ok(bytes)
}

fn validate_replacements(files: &[(&str, &[u8])]) -> io::Result<()> {
    if files.is_empty() || files.len() > MAX_ENTRIES {
        return Err(io::Error::other("invalid certificate generation size"));
    }
    for (index, (name, bytes)) in files.iter().enumerate() {
        super::validate_file_name(OsStr::new(name))?;
        if bytes.len() as u64 > MAX_MATERIAL_BYTES {
            return Err(io::Error::other("certificate material is too large"));
        }
        if files[..index].iter().any(|(prior, _)| prior == name) {
            return Err(io::Error::other("duplicate certificate file name"));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod implementation {
    use super::*;
    use crate::managed_fs::platform::{c_name, open_at, open_directory};
    use std::ffi::{CStr, CString, OsString};
    use std::io::Write;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawFd, IntoRawFd},
    };
    use std::sync::atomic::Ordering;

    const DIRECTORY_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const FILE_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;

    struct DirectoryStream(*mut libc::DIR);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            // SAFETY: fdopendir transferred its owned descriptor to this stream.
            unsafe { libc::closedir(self.0) };
        }
    }

    fn entry_names(directory: &File) -> io::Result<Vec<OsString>> {
        // A fresh open file description avoids sharing the directory offset.
        let raw = open_at(directory, c".", DIRECTORY_FLAGS)?.into_raw_fd();
        let pointer = unsafe { libc::fdopendir(raw) };
        if pointer.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(raw) };
            return Err(error);
        }
        let stream = DirectoryStream(pointer);
        let mut names = Vec::new();
        loop {
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(0) {
                    Ok(names)
                } else {
                    Err(error)
                };
            }
            // SAFETY: readdir supplies a NUL-terminated name until its next call.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if names.len() == MAX_ENTRIES {
                return Err(io::Error::other(
                    "certificate directory has too many entries",
                ));
            }
            let name = OsStr::from_bytes(bytes);
            super::super::validate_file_name(name)?;
            names.push(name.to_owned());
        }
    }

    fn regular_file(directory: &File, name: &CStr) -> io::Result<File> {
        let file = open_at(directory, name, FILE_FLAGS)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other(
                "certificate directory may contain regular files only",
            ));
        }
        Ok(file)
    }

    fn cleanup_flat(parent: &File, name: &CStr, directory: &File) {
        // Never recurse through unexpected directories or follow a link. A
        // failed cleanup may leave private staging data; publication succeeded.
        if let Ok(names) = entry_names(directory) {
            for name in names {
                if let Ok(name) = c_name(&name) {
                    unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
                }
            }
        }
        unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
    }

    struct StagingDirectory {
        parent: File,
        name: CString,
        directory: File,
        previous: Option<File>,
    }
    impl Drop for StagingDirectory {
        fn drop(&mut self) {
            cleanup_flat(
                &self.parent,
                &self.name,
                self.previous.as_ref().unwrap_or(&self.directory),
            );
        }
    }
    impl StagingDirectory {
        fn new(parent: &File) -> io::Result<Self> {
            let name = CString::new(format!(
                ".sbctl-generation.{}.{}.tmp",
                std::process::id(),
                super::super::TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap();
            let parent = parent.try_clone()?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let directory = match open_at(&parent, &name, DIRECTORY_FLAGS) {
                Ok(directory) => directory,
                Err(error) => {
                    unsafe {
                        libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR)
                    };
                    return Err(error);
                }
            };
            Ok(Self {
                parent,
                name,
                directory,
                previous: None,
            })
        }

        fn commit(&mut self, name: &CStr, previous: File) -> io::Result<()> {
            self.directory.sync_all()?;
            let current = open_at(&self.parent, name, DIRECTORY_FLAGS)?.metadata()?;
            let expected = previous.metadata()?;
            if (current.dev(), current.ino()) != (expected.dev(), expected.ino()) {
                return Err(io::Error::other(
                    "certificate directory changed during staging",
                ));
            }
            if unsafe {
                libc::renameat2(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    self.parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            // The directory exchange is the publication point. Drop cleans the
            // old directory, never the directory now visible at the destination.
            self.previous = Some(previous);
            let _ = self.parent.sync_all();
            Ok(())
        }
    }

    pub(crate) fn replace_files_with_marker(
        directory: &Path,
        files: &[(&str, &[u8])],
        marker: Option<&str>,
        directory_policy: impl Fn(&File) -> io::Result<()>,
        file_policy: impl Fn(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        validate_replacements(files)?;
        if let Some(marker) = marker {
            super::super::validate_file_name(OsStr::new(marker))?;
            if files.iter().any(|(name, _)| *name == marker) {
                return Err(io::Error::other("refresh marker overlaps material"));
            }
        }
        let (parent_path, name) = super::super::split_file(directory)?;
        // This lock is separate from the deployment operation lock, which
        // callers such as installation and mode switching may already hold.
        let parent = open_directory(parent_path, false)?;
        let lock = crate::managed_fs::platform::open_lock_in(&parent)?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let name = c_name(name)?;
        let previous = open_at(&parent, &name, DIRECTORY_FLAGS)?;
        let mut staged = StagingDirectory::new(&parent)?;
        let entries = entry_names(&previous)?;
        let retained = entries
            .iter()
            .filter(|entry| {
                !files
                    .iter()
                    .any(|(name, _)| OsStr::new(name) == entry.as_os_str())
            })
            .count();
        let extra_marker =
            usize::from(marker.is_some_and(|marker| !entries.iter().any(|name| name == marker)));
        if retained + files.len() + extra_marker > MAX_ENTRIES {
            return Err(io::Error::other(
                "certificate generation has too many entries",
            ));
        }
        for entry in entries {
            let entry_name = c_name(&entry)?;
            let source = regular_file(&previous, &entry_name)?;
            if files.iter().any(|(name, _)| OsStr::new(name) == entry) {
                continue;
            }
            if unsafe {
                libc::linkat(
                    previous.as_raw_fd(),
                    entry_name.as_ptr(),
                    staged.directory.as_raw_fd(),
                    entry_name.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            // linkat without AT_SYMLINK_FOLLOW never follows a source link.
            // Re-open and compare to the pinned inode to detect a leaf swap.
            let retained = regular_file(&staged.directory, &entry_name)?;
            let expected = source.metadata()?;
            let actual = retained.metadata()?;
            if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino()) {
                return Err(io::Error::other("certificate entry changed during staging"));
            }
        }
        for (name, bytes) in files {
            let name = c_name(OsStr::new(name))?;
            let mut file = open_at(
                &staged.directory,
                &name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )?;
            file.write_all(bytes)?;
            file_policy(&file)?;
            file.sync_all()?;
        }
        directory_policy(&staged.directory)?;
        let same_metadata = |left: &File, right: &File| -> io::Result<bool> {
            let left = left.metadata()?;
            let right = right.metadata()?;
            Ok((left.uid(), left.gid(), left.mode()) == (right.uid(), right.gid(), right.mode()))
        };
        let mut unchanged = same_metadata(&previous, &staged.directory)?;
        for (name, bytes) in files {
            let name = c_name(OsStr::new(name))?;
            let old = match regular_file(&previous, &name) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    unchanged = false;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let new = regular_file(&staged.directory, &name)?;
            unchanged &=
                same_metadata(&old, &new)? && read_material(old).is_ok_and(|old| old == *bytes);
        }
        if unchanged {
            return Ok(());
        }
        if let Some(marker) = marker {
            let marker = c_name(OsStr::new(marker))?;
            if unsafe { libc::unlinkat(staged.directory.as_raw_fd(), marker.as_ptr(), 0) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(error);
                }
            }
            let file = open_at(
                &staged.directory,
                &marker,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )?;
            file.sync_all()?;
        }
        staged.commit(&name, previous)
    }

    #[cfg(test)]
    fn replace_files(
        directory: &Path,
        files: &[(&str, &[u8])],
        directory_policy: impl Fn(&File) -> io::Result<()>,
        file_policy: impl Fn(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        replace_files_with_marker(directory, files, None, directory_policy, file_policy)
    }

    pub(crate) fn read_pair(
        directory: &Path,
        first: &str,
        second: &str,
    ) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let directory = open_directory(directory, false)?;
        // Open both descriptors before reading: once open, cleanup of the old
        // generation cannot remove the material from an in-flight read.
        let first = regular_file(&directory, &c_name(OsStr::new(first))?)?;
        let second = regular_file(&directory, &c_name(OsStr::new(second))?)?;
        Ok((read_material(first)?, read_material(second)?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::Cell;
        use std::fs;
        use std::os::unix::fs::{PermissionsExt, symlink};

        fn seed() -> (tempfile::TempDir, std::path::PathBuf) {
            let fixture = tempfile::tempdir().unwrap();
            let directory = fixture.path().join("host");
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("cert"), b"old").unwrap();
            fs::write(directory.join("key"), b"old").unwrap();
            (fixture, directory)
        }

        #[test]
        fn second_file_or_final_policy_failure_preserves_the_complete_old_generation() {
            for directory_failure in [false, true] {
                let (fixture, directory) = seed();
                let inode = fs::metadata(&directory).unwrap().ino();
                let writes = Cell::new(0);
                let result = replace_files(
                    &directory,
                    &[("cert", b"new"), ("key", b"new")],
                    |_| {
                        if directory_failure {
                            Err(io::Error::other("injected directory policy error"))
                        } else {
                            Ok(())
                        }
                    },
                    |_| {
                        writes.set(writes.get() + 1);
                        if !directory_failure && writes.get() == 2 {
                            Err(io::Error::other("injected second file policy error"))
                        } else {
                            Ok(())
                        }
                    },
                );
                assert!(result.is_err());
                assert_eq!(fs::metadata(&directory).unwrap().ino(), inode);
                assert_eq!(
                    read_pair(&directory, "cert", "key").unwrap(),
                    (b"old".to_vec(), b"old".to_vec())
                );
                assert_eq!(
                    fs::read_dir(fixture.path()).unwrap().count(),
                    2,
                    "only host and publication lock remain"
                );
            }
        }

        #[test]
        fn invalid_names_duplicate_entries_and_oversized_material_preserve_the_old_pair() {
            let (_fixture, directory) = seed();
            let oversized = vec![0; MAX_MATERIAL_BYTES as usize + 1];
            for files in [
                vec![("../outside", b"new".as_slice())],
                vec![("cert", b"one".as_slice()), ("cert", b"two".as_slice())],
                vec![("cert", oversized.as_slice())],
            ] {
                assert!(replace_files(&directory, &files, |_| Ok(()), |_| Ok(())).is_err());
                assert_eq!(
                    read_pair(&directory, "cert", "key").unwrap(),
                    (b"old".to_vec(), b"old".to_vec())
                );
            }
            fs::write(directory.join("key"), &oversized).unwrap();
            assert!(
                read_pair(&directory, "cert", "key").is_err(),
                "oversized pinned material is bounded during read too"
            );
        }

        #[test]
        fn exchange_preserves_auxiliary_files_and_already_open_reader_handles() {
            let (fixture, directory) = seed();
            fs::write(directory.join("cert.pem"), b"other certificate pair").unwrap();
            let retained_inode = fs::metadata(directory.join("cert.pem")).unwrap().ino();
            let old = open_directory(&directory, false).unwrap();
            let cert = regular_file(&old, c"cert").unwrap();
            let key = regular_file(&old, c"key").unwrap();
            replace_files(
                &directory,
                &[("cert", b"new"), ("key", b"new")],
                |file| file.set_permissions(fs::Permissions::from_mode(0o750)),
                |file| file.set_permissions(fs::Permissions::from_mode(0o640)),
            )
            .unwrap();
            assert_eq!(
                (read_material(cert).unwrap(), read_material(key).unwrap()),
                (b"old".to_vec(), b"old".to_vec())
            );
            assert_eq!(
                read_pair(&directory, "cert", "key").unwrap(),
                (b"new".to_vec(), b"new".to_vec())
            );
            assert_eq!(
                fs::metadata(directory.join("cert.pem")).unwrap().ino(),
                retained_inode
            );
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o750);
            assert_eq!(
                fs::metadata(directory.join("key")).unwrap().mode() & 0o777,
                0o640
            );
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
        }

        #[test]
        fn unchanged_material_and_policy_keep_directory_and_watchers_in_place() {
            let (fixture, directory) = seed();
            let files = [("cert", b"new".as_slice()), ("key", b"new".as_slice())];
            let directory_policy =
                |file: &File| file.set_permissions(fs::Permissions::from_mode(0o750));
            let file_policy = |file: &File| file.set_permissions(fs::Permissions::from_mode(0o640));
            replace_files(&directory, &files, directory_policy, file_policy).unwrap();
            let inode = fs::metadata(&directory).unwrap().ino();
            replace_files(&directory, &files, directory_policy, file_policy).unwrap();
            assert_eq!(fs::metadata(&directory).unwrap().ino(), inode);
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
        }

        #[test]
        fn pending_refresh_is_committed_with_the_pair_and_survives_unchanged_retries() {
            let (_fixture, directory) = seed();
            let files = [("cert", b"new".as_slice()), ("key", b"new".as_slice())];
            let publish = || {
                replace_files_with_marker(
                    &directory,
                    &files,
                    Some(".pending"),
                    |_| Ok(()),
                    |_| Ok(()),
                )
            };
            publish().unwrap();
            let inode = fs::metadata(&directory).unwrap().ino();
            assert_eq!(
                fs::metadata(directory.join(".pending")).unwrap().mode() & 0o777,
                0o600
            );
            publish().unwrap();
            assert!(directory.join(".pending").is_file());
            assert_eq!(fs::metadata(&directory).unwrap().ino(), inode);
            fs::remove_file(directory.join(".pending")).unwrap();
            publish().unwrap();
            assert!(
                !directory.join(".pending").exists(),
                "acknowledged unchanged material needs no new refresh"
            );
        }

        #[test]
        fn commit_refuses_a_destination_replaced_during_staging() {
            let (fixture, directory) = seed();
            let saved = fixture.path().join("saved");
            let result = replace_files(
                &directory,
                &[("cert", b"new"), ("key", b"new")],
                |_| {
                    fs::rename(&directory, &saved)?;
                    fs::create_dir(&directory)?;
                    Ok(())
                },
                |_| Ok(()),
            );
            assert!(result.is_err());
            assert_eq!(
                read_pair(&saved, "cert", "key").unwrap(),
                (b"old".to_vec(), b"old".to_vec())
            );
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
        }

        #[test]
        fn publication_and_cleanup_remain_in_pinned_parent_after_path_replacement() {
            for fail in [false, true] {
                let fixture = tempfile::tempdir().unwrap();
                let parent = fixture.path().join("parent");
                let moved = fixture.path().join("moved");
                let outside = fixture.path().join("outside");
                fs::create_dir(&parent).unwrap();
                fs::create_dir(&outside).unwrap();
                fs::create_dir(parent.join("host")).unwrap();
                fs::write(parent.join("host/cert"), b"old").unwrap();
                fs::write(parent.join("host/key"), b"old").unwrap();
                fs::write(outside.join("sentinel"), b"private").unwrap();
                let result = replace_files(
                    &parent.join("host"),
                    &[("cert", b"new"), ("key", b"new")],
                    |_| {
                        fs::rename(&parent, &moved)?;
                        symlink(&outside, &parent)?;
                        if fail {
                            Err(io::Error::other("injected policy failure"))
                        } else {
                            Ok(())
                        }
                    },
                    |_| Ok(()),
                );
                assert_eq!(result.is_err(), fail);
                let expected = if fail { b"old" } else { b"new" };
                assert_eq!(
                    read_pair(&moved.join("host"), "cert", "key").unwrap(),
                    (expected.to_vec(), expected.to_vec())
                );
                assert_eq!(fs::read_dir(&moved).unwrap().count(), 2);
                assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
                assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"private");
            }
        }

        #[test]
        fn linked_or_non_flat_content_is_rejected_without_touching_old_material() {
            for link in [false, true] {
                let (fixture, directory) = seed();
                let sentinel = fixture.path().join("sentinel");
                fs::write(&sentinel, b"private").unwrap();
                if link {
                    symlink(&sentinel, directory.join("extra")).unwrap();
                } else {
                    fs::create_dir(directory.join("extra")).unwrap();
                }
                assert!(
                    replace_files(
                        &directory,
                        &[("cert", b"new"), ("key", b"new")],
                        |_| Ok(()),
                        |_| Ok(())
                    )
                    .is_err()
                );
                assert_eq!(
                    read_pair(&directory, "cert", "key").unwrap(),
                    (b"old".to_vec(), b"old".to_vec())
                );
                assert_eq!(fs::read(&sentinel).unwrap(), b"private");
                assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
            }
        }

        #[test]
        fn concurrent_readers_never_accept_mixed_generations() {
            let (_fixture, directory) = seed();
            std::thread::scope(|scope| {
                let writer = scope.spawn(|| {
                    for generation in 0..64 {
                        let bytes = generation.to_string();
                        replace_files(
                            &directory,
                            &[("cert", bytes.as_bytes()), ("key", bytes.as_bytes())],
                            |_| Ok(()),
                            |_| Ok(()),
                        )
                        .unwrap();
                    }
                });
                let mut reads = 0;
                loop {
                    match read_pair(&directory, "cert", "key") {
                        Ok((cert, key)) => {
                            assert_eq!(cert, key);
                            reads += 1;
                        }
                        Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
                    }
                    if writer.is_finished() {
                        break;
                    }
                }
                writer.join().unwrap();
                assert!(reads > 0);
            });
            let (cert, key) = read_pair(&directory, "cert", "key").unwrap();
            assert_eq!(cert, key);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod implementation {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    fn existing_directory(path: &Path) -> io::Result<()> {
        for path in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
            let metadata = fs::symlink_metadata(path)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(io::Error::other(
                        "certificate directory must not be a reparse point",
                    ));
                }
            }
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("certificate directory must not be a link"));
            }
        }
        Ok(())
    }

    struct StagingDirectory(PathBuf);
    impl Drop for StagingDirectory {
        fn drop(&mut self) {
            if let Ok(entries) = fs::read_dir(&self.0) {
                for entry in entries.flatten() {
                    let _ = fs::remove_file(entry.path());
                }
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    pub(crate) fn replace_files_with_marker(
        directory: &Path,
        files: &[(&str, &[u8])],
        marker: Option<&str>,
        directory_policy: impl Fn(&File) -> io::Result<()>,
        file_policy: impl Fn(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        validate_replacements(files)?;
        existing_directory(directory)?;
        let (parent, _) = super::super::split_file(directory)?;
        let lock = super::super::open_lock(parent)?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let temporary = parent.join(format!(
            ".sbctl-generation.{}.{}.tmp",
            std::process::id(),
            super::super::TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let directory_builder = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut directory_builder = fs::DirBuilder::new();
                directory_builder.mode(0o700);
                directory_builder
            }
            #[cfg(not(unix))]
            {
                fs::DirBuilder::new()
            }
        };
        directory_builder.create(&temporary)?;
        let mut staged = StagingDirectory(temporary);
        let mut count = 0;
        for entry in fs::read_dir(directory)? {
            count += 1;
            if count > MAX_ENTRIES {
                return Err(io::Error::other(
                    "certificate directory has too many entries",
                ));
            }
            let entry = entry?;
            let name = entry.file_name();
            super::super::validate_file_name(&name)?;
            if !entry.file_type()?.is_file() {
                return Err(io::Error::other(
                    "certificate directory may contain regular files only",
                ));
            }
            if !files
                .iter()
                .any(|(replacement, _)| OsStr::new(replacement) == name)
            {
                fs::hard_link(entry.path(), staged.0.join(name))?;
            }
        }
        for (name, bytes) in files {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(staged.0.join(name))?;
            file.write_all(bytes)?;
            file_policy(&file)?;
            file.sync_all()?;
        }
        // Windows cannot open directories through File::open. Ownership is a
        // no-op in this development adapter; Unix development applies policy.
        #[cfg(unix)]
        directory_policy(&File::open(&staged.0)?)?;
        #[cfg(not(unix))]
        let _ = directory_policy;
        if let Some(marker) = marker {
            super::super::validate_file_name(OsStr::new(marker))?;
            if files.iter().any(|(name, _)| *name == marker) {
                return Err(io::Error::other("refresh marker overlaps material"));
            }
            let path = staged.0.join(marker);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path)?.sync_all()?;
        }
        let backup = staged.0.with_extension("previous");
        if backup.exists() {
            return Err(io::Error::other("certificate backup already exists"));
        }
        fs::rename(directory, &backup)?;
        if let Err(error) = fs::rename(&staged.0, directory) {
            fs::rename(&backup, directory).map_err(|recovery| {
                io::Error::other(format!("{error}; directory recovery failed: {recovery}"))
            })?;
            return Err(error);
        }
        staged.0 = backup;
        Ok(())
    }

    pub(crate) fn read_pair(
        directory: &Path,
        first: &str,
        second: &str,
    ) -> io::Result<(Vec<u8>, Vec<u8>)> {
        existing_directory(directory)?;
        let read = |name: &str| {
            super::super::validate_file_name(OsStr::new(name))?;
            let path = directory.join(name);
            if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(io::Error::other("certificate material must not be a link"));
            }
            read_material(File::open(path)?)
        };
        Ok((read(first)?, read(second)?))
    }
}

pub(crate) use implementation::{read_pair, replace_files_with_marker as replace_files};
