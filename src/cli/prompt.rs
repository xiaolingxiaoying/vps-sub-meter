//! Shared console prompts. These are used by both the non-interactive
//! install path and the interactive menu, so they live apart from `cli::menu`.

use crate::cli::args::CliManagedProtocol;
use std::io::{self, BufRead, Write};

pub(crate) fn select_protocols(
    disabled: &[CliManagedProtocol],
) -> Result<Vec<sbctl::config::ManagedProtocol>, sbctl::config::ConfigError> {
    let defaults = [
        CliManagedProtocol::VlessReality,
        CliManagedProtocol::VmessWebsocket,
        CliManagedProtocol::Hysteria2,
        CliManagedProtocol::Tuic,
        CliManagedProtocol::Anytls,
    ];
    let mut selected = Vec::new();
    for protocol in defaults {
        let disabled_by_flag = disabled
            .iter()
            .any(|disabled| std::mem::discriminant(disabled) == std::mem::discriminant(&protocol));
        if !disabled_by_flag {
            selected.push(protocol.into());
        }
    }
    Ok(selected)
}

pub(crate) fn protocol_ports(
    vless_reality: Option<u16>,
    vmess_websocket: Option<u16>,
    hysteria2: Option<u16>,
    tuic: Option<u16>,
    anytls: Option<u16>,
) -> sbctl::config::ProtocolPorts {
    sbctl::config::ProtocolPorts {
        vless_reality,
        vmess_websocket,
        hysteria2,
        tuic,
        anytls,
    }
}

pub(crate) fn secret(label: &str) -> io::Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    #[cfg(unix)]
    struct EchoGuard(libc::termios);
    #[cfg(unix)]
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0);
            }
        }
    }
    #[cfg(unix)]
    let _guard = {
        use std::io::IsTerminal;
        if io::stdin().is_terminal() {
            let mut old = std::mem::MaybeUninit::<libc::termios>::uninit();
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, old.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let old = unsafe { old.assume_init() };
            let mut hidden = old;
            hidden.c_lflag &= !libc::ECHO;
            if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &hidden) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Some(EchoGuard(old))
        } else {
            None
        }
    };
    #[cfg(not(unix))]
    {
        Err(io::Error::other(
            "请在 Linux VPS 使用邮件向导；开发环境可用 email configure 编辑配置",
        ))
    }
    #[cfg(unix)]
    {
        let mut value = String::new();
        io::stdin().read_line(&mut value)?;
        println!();
        Ok(value.trim_end_matches(['\r', '\n']).to_owned())
    }
}

pub(crate) struct ConsolePrompts;

impl sbctl::wizard::Prompts for ConsolePrompts {
    fn ask(&mut self, label: &str, default: Option<&str>) -> io::Result<String> {
        print!("{label}");
        if let Some(default) = default {
            print!(" [{}]", default);
        }
        print!(": ");
        io::stdout().flush()?;
        let mut answer = String::new();
        let read = io::stdin().read_line(&mut answer)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "wizard input ended before the prompt was answered",
            ));
        }
        Ok(answer.trim().to_owned())
    }

    fn report(&mut self, message: &str) {
        println!("{message}");
    }

    fn confirm(&mut self, question: &str, default: bool) -> io::Result<bool> {
        confirm_with_io(
            &mut io::stdin().lock(),
            &mut io::stdout().lock(),
            question,
            default,
        )
    }
}

fn confirm_with_io(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
    default: bool,
) -> io::Result<bool> {
    loop {
        write!(
            output,
            "{question}（1 是 / 2 否）[{}]: ",
            if default { "1" } else { "2" }
        )?;
        output.flush()?;
        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "wizard input ended before the confirmation was answered",
            ));
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "1" | "y" | "yes" | "是" | "确认" => return Ok(true),
            "2" | "n" | "no" | "否" | "取消" => return Ok(false),
            _ => writeln!(output, "请输入 1 / 是 或 2 / 否；直接回车使用默认值。")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_retries_invalid_input_and_accepts_chinese_answers() {
        for (answers, expected) in [
            ("oops\n是\n", true),
            ("?\n取消\n", false),
            ("?\n1\n", true),
            ("?\n2\n", false),
        ] {
            let mut input = io::Cursor::new(answers);
            let mut output = Vec::new();
            assert_eq!(
                confirm_with_io(&mut input, &mut output, "应用?", false).unwrap(),
                expected
            );
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains("请输入"));
            assert_eq!(output.matches("应用?").count(), 2);
        }
    }

    #[test]
    fn confirmation_uses_explicit_default_but_never_treats_eof_as_acceptance() {
        for default in [false, true] {
            assert_eq!(
                confirm_with_io(
                    &mut io::Cursor::new("\n"),
                    &mut Vec::new(),
                    "应用?",
                    default
                )
                .unwrap(),
                default
            );
            let error =
                confirm_with_io(&mut io::Cursor::new(""), &mut Vec::new(), "应用?", default)
                    .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
    }
}
