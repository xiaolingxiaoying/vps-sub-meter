//! Runtime adapters shared by production code and isolated acceptance fixtures.
//!
//! The application deliberately talks to the host through this small boundary:
//! production uses the real clock and filesystem root, while tests can supply a
//! fixed instant and a temporary root containing synthetic host files.

use chrono::{DateTime, Utc};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};

pub trait Clock {
    fn now_utc(&self) -> DateTime<Utc>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct Runtime<C> {
    root: PathBuf,
    clock: C,
    allow_live_commands: bool,
}

impl Runtime<SystemClock> {
    pub fn live(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            clock: SystemClock,
            allow_live_commands: root == Path::new("/"),
        }
    }
}

impl Runtime<FixedClock> {
    pub fn fixture(root: &Path, now: DateTime<Utc>) -> Self {
        Self {
            root: root.to_owned(),
            clock: FixedClock(now),
            allow_live_commands: false,
        }
    }
}

impl<C: Clock> Runtime<C> {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn now_utc(&self) -> DateTime<Utc> {
        self.clock.now_utc()
    }

    /// Read a host-relative file without allowing an absolute path to escape
    /// the fixture root.
    pub fn read_to_string(&self, relative: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(self.host_path(relative)?)
    }

    /// Run a host command. Alternate roots provide `usr/bin/<program>` or
    /// `usr/sbin/<program>` themselves; only root `/` may fall back to PATH.
    pub fn run_command(&self, program: &str, args: &[&str]) -> io::Result<ExitStatus> {
        self.command(program, args)?.status()
    }

    /// Run a host command and capture its standard output and error. Used when
    /// a diagnostic is needed without printing secrets; the captured text is
    /// returned separately so callers can redact or summarize it.
    pub fn run_command_output(
        &self,
        program: &str,
        args: &[&str],
    ) -> io::Result<(ExitStatus, String)> {
        let output = self.command(program, args)?.output()?;
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok((output.status, combined))
    }

    /// Shared process boundary for commands that affect host state. Alternate
    /// roots must supply their own command, even when using the real clock.
    pub(crate) fn command(&self, program: &str, args: &[&str]) -> io::Result<Command> {
        if program.is_empty()
            || Path::new(program).is_absolute()
            || program.contains(['/', '\\'])
            || !matches!(
                Path::new(program).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime command must be a simple program name",
            ));
        }
        let rooted = ["usr/bin", "usr/sbin"].into_iter().find_map(|directory| {
            let candidate = self.root.join(directory).join(program);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let candidate = candidate.with_file_name(format!("{program}.cmd"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            None
        });
        let executable = match rooted {
            Some(path) => path,
            None if self.allow_live_commands => PathBuf::from(program),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("fixture command is missing: {program}"),
                ));
            }
        };
        let mut command = Command::new(executable);
        command.args(args);
        Ok(command)
    }

    fn host_path(&self, relative: impl AsRef<Path>) -> io::Result<PathBuf> {
        let relative = relative.as_ref();
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host path must be relative to the runtime root",
            ));
        }
        Ok(self.root.join(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FixedClock, Runtime};
    use chrono::{TimeZone, Utc};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn fixture_runtime_reads_host_files_and_returns_a_fixed_instant() {
        let fixture = TempDir::new().unwrap();
        fs::create_dir_all(fixture.path().join("sys/class/net/ens3/statistics")).unwrap();
        fs::write(
            fixture
                .path()
                .join("sys/class/net/ens3/statistics/rx_bytes"),
            "123\n",
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2025, 2, 3, 4, 5, 6).unwrap();
        let runtime = Runtime::fixture(fixture.path(), now);

        assert_eq!(runtime.now_utc(), now);
        assert_eq!(
            runtime
                .read_to_string("sys/class/net/ens3/statistics/rx_bytes")
                .unwrap(),
            "123\n"
        );
    }

    #[test]
    #[cfg(unix)]
    fn fixture_runtime_executes_a_rooted_command_without_touching_the_live_host() {
        let fixture = TempDir::new().unwrap();
        let runtime = Runtime::fixture(fixture.path(), Utc::now());
        for (directory, program) in [("usr/bin", "systemctl"), ("usr/sbin", "useradd")] {
            let command = fixture.path().join(directory).join(program);
            fs::create_dir_all(command.parent().unwrap()).unwrap();
            fs::write(&command, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(
                runtime
                    .run_command(program, &["--fixture-only"])
                    .unwrap()
                    .success()
            );
        }
        assert!(!fixture.path().join("command-output").exists());
    }

    #[test]
    fn fixture_runtime_rejects_host_escape_paths_and_missing_commands() {
        let fixture = TempDir::new().unwrap();
        let runtime = Runtime::fixture(fixture.path(), Utc::now());

        assert!(runtime.read_to_string("../outside").is_err());
        assert!(runtime.read_to_string("/etc/passwd").is_err());
        assert!(runtime.run_command("missing-systemctl", &[]).is_err());
        for invalid in [
            ".",
            "..",
            "../systemctl",
            "usr/bin/systemctl",
            "usr\\bin\\systemctl",
        ] {
            assert_eq!(
                runtime.command(invalid, &[]).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn live_runtime_with_an_alternate_root_requires_its_own_host_commands() {
        let fixture = TempDir::new().unwrap();
        let runtime = Runtime::live(fixture.path());
        for program in [
            "systemctl",
            "sysctl",
            "certbot",
            "ufw",
            "useradd",
            "groupadd",
            "usermod",
        ] {
            assert_eq!(
                runtime.command(program, &[]).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
        }
    }

    #[test]
    fn fixed_clock_is_a_reusable_clock_adapter() {
        let instant = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert_eq!(FixedClock(instant).now_utc(), instant);
    }
}
