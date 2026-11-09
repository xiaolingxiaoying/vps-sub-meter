//! Data-plane command boundary. Client validators select their own CoreKind;
//! they must never inherit the selected server engine implicitly.
use std::io::Write;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

pub mod presets;
pub mod xray;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoreKind {
    #[default]
    SingBox,
    Xray,
}

impl std::fmt::Display for CoreKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("could not prepare or execute core configuration check: {0}")]
    Io(#[from] std::io::Error),
    // Never include the core's output: configuration diagnostics can contain
    // credentials, private keys or user-authored configuration fragments.
    #[error("{core} configuration check exited with {status}")]
    Check {
        core: CoreKind,
        status: std::process::ExitStatus,
    },
}

impl CoreKind {
    /// Arguments are structured data, never a shell command. The config path
    /// is appended by the caller, including when it contains spaces.
    pub fn check_arguments(self) -> &'static [&'static str] {
        match self {
            Self::SingBox => &["check", "-c"],
            Self::Xray => &["run", "-test", "-c"],
        }
    }

    pub fn run_arguments(self) -> &'static [&'static str] {
        &["run", "-c"]
    }

    /// Validate an already-rendered document without activating any service.
    /// This validates configuration only, not executable provenance or identity;
    /// candidate downloads must be authenticated separately before this call.
    pub fn check_config(self, binary: &Path, document: &str) -> Result<(), CoreError> {
        // Xray's auto format detection requires a recognized file extension.
        let mut temporary = tempfile::Builder::new().suffix(".json").tempfile()?;
        temporary.write_all(document.as_bytes())?;
        temporary.flush()?;
        // Reopenable on Windows as well as Unix; keep the private TempPath alive.
        let path = temporary.into_temp_path();
        let output = Command::new(binary)
            .args(self.check_arguments())
            .arg(&path)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(CoreError::Check {
                core: self,
                status: output.status,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_names_are_stable_and_unknown_engines_are_rejected() {
        assert_eq!(CoreKind::default(), CoreKind::SingBox);
        for (name, kind) in [("sing-box", CoreKind::SingBox), ("xray", CoreKind::Xray)] {
            assert_eq!(
                serde_json::from_str::<CoreKind>(&format!("\"{name}\"")).unwrap(),
                kind
            );
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{name}\""));
        }
        assert!(serde_json::from_str::<CoreKind>("\"custom-shell\"").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn configuration_checks_use_engine_specific_arguments_and_redact_output() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("validator with spaces");
        std::fs::write(&binary, "#!/bin/sh\nif [ \"$1\" = check ]; then\n  [ \"$2\" = -c ] && [ -s \"$3\" ] || exit 7\n  exit 0\nfi\n[ \"$1\" = run ] && [ \"$2\" = -test ] && [ \"$3\" = -c ] && [ -s \"$4\" ] || exit 8\necho PRIVATE-CREDENTIAL >&2\nexit 23\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        CoreKind::SingBox.check_config(&binary, "{}").unwrap();
        let error = CoreKind::Xray.check_config(&binary, "{}").unwrap_err();
        assert!(error.to_string().contains("23"));
        assert!(!error.to_string().contains("PRIVATE-CREDENTIAL"));
    }
}
