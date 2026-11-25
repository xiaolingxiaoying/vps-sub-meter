//! Interactive prompts that allow returning to the previous menu with `0`
//! at every step instead of trapping the operator until Ctrl+C.

use std::io::{self, BufRead, Write};

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("已输入 0 取消并保留原配置")]
    Cancelled,
    #[error("输入流提前结束")]
    Eof,
    #[error("终端读写失败: {0}")]
    Io(#[from] io::Error),
}

pub type PromptResult<T> = Result<T, PromptError>;

pub trait PromptIo {
    fn ask(&mut self, label: &str, default: Option<&str>) -> PromptResult<String>;
    fn report(&mut self, message: &str);
    fn confirm(&mut self, question: &str, default: bool) -> PromptResult<bool>;
    fn pause(&mut self, message: &str);
}

pub struct ConsoleIo;

impl PromptIo for ConsoleIo {
    fn ask(&mut self, label: &str, default: Option<&str>) -> PromptResult<String> {
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout().lock();
        read_line_with_back(&mut stdin, &mut stdout, label, default)
    }

    fn report(&mut self, message: &str) {
        println!("{message}");
    }

    fn confirm(&mut self, question: &str, default: bool) -> PromptResult<bool> {
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout().lock();
        confirm_with_io(&mut stdin, &mut stdout, question, default)
    }

    fn pause(&mut self, message: &str) {
        print!("\n{message}");
        let _ = io::stdout().flush();
        let mut ignored = String::new();
        let _ = io::stdin().read_line(&mut ignored);
    }
}

pub struct BufferedIo<R, W> {
    pub reader: R,
    pub writer: W,
}

impl<R: BufRead, W: Write> BufferedIo<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R: BufRead, W: Write> PromptIo for BufferedIo<R, W> {
    fn ask(&mut self, label: &str, default: Option<&str>) -> PromptResult<String> {
        read_line_with_back(&mut self.reader, &mut self.writer, label, default)
    }

    fn report(&mut self, message: &str) {
        let _ = writeln!(self.writer, "{message}");
    }

    fn confirm(&mut self, question: &str, default: bool) -> PromptResult<bool> {
        confirm_with_io(&mut self.reader, &mut self.writer, question, default)
    }

    fn pause(&mut self, _message: &str) {}
}

fn read_line_with_back(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: Option<&str>,
) -> PromptResult<String> {
    write!(output, "{label} (0 返回)")?;
    if let Some(def) = default {
        write!(output, " [{def}]")?;
    }
    write!(output, ": ")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Err(PromptError::Eof);
    }
    let trimmed = line.trim();
    if trimmed == "0" {
        return Err(PromptError::Cancelled);
    }
    Ok(trimmed.to_owned())
}

fn confirm_with_io(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
    default: bool,
) -> PromptResult<bool> {
    loop {
        write!(
            output,
            "{question} (1 是 / 2 否 / 0 返回) [{}]: ",
            if default { "1" } else { "2" }
        )?;
        output.flush()?;
        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            return Err(PromptError::Eof);
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "0" => return Err(PromptError::Cancelled),
            "" => return Ok(default),
            "1" | "y" | "yes" | "是" | "确认" => return Ok(true),
            "2" | "n" | "no" | "否" | "取消" => return Ok(false),
            _ => writeln!(output, "请输入 1(是)、2(否) 或 0(返回)")?,
        }
    }
}

pub fn ask_text(
    io: &mut impl PromptIo,
    label: &str,
    default: Option<&str>,
    allow_empty: bool,
) -> PromptResult<String> {
    loop {
        let answer = io.ask(label, default)?;
        if answer.is_empty() {
            if let Some(def) = default {
                return Ok(def.to_owned());
            }
            if allow_empty {
                return Ok(String::new());
            }
            io.report("此项不能为空，或输入 0 返回。");
            continue;
        }
        return Ok(answer);
    }
}

pub fn ask_parsed<T>(
    io: &mut impl PromptIo,
    label: &str,
    default: Option<&str>,
    parse: impl Fn(&str) -> Result<T, String>,
) -> PromptResult<T> {
    loop {
        let raw = io.ask(label, default)?;
        let candidate = if raw.is_empty() {
            match default {
                Some(def) => def.to_owned(),
                None => {
                    io.report("此项必填，请输入有效值或输入 0 返回。");
                    continue;
                }
            }
        } else {
            raw
        };
        match parse(&candidate) {
            Ok(v) => return Ok(v),
            Err(msg) => io.report(&format!("输入无效: {msg} (输入 0 可返回)")),
        }
    }
}

pub fn ask_menu_choice(
    io: &mut impl PromptIo,
    prompt: &str,
    max_index: usize,
) -> PromptResult<usize> {
    loop {
        let raw = match io.ask(prompt, Some("0")) {
            Ok(v) => v,
            Err(PromptError::Cancelled) => return Ok(0),
            Err(e) => return Err(e),
        };
        if raw.is_empty() {
            return Ok(0);
        }
        match raw.parse::<usize>() {
            Ok(n) if n <= max_index => return Ok(n),
            _ => io.report(&format!("请输入 0..={max_index} 之间的编号")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_cancels_every_prompt_without_trapping_user() {
        let mut io = BufferedIo::new(io::Cursor::new("0\n"), Vec::new());
        assert!(matches!(
            ask_text(&mut io, "域名", Some("example.com"), false),
            Err(PromptError::Cancelled)
        ));

        let mut io = BufferedIo::new(io::Cursor::new("0\n"), Vec::new());
        assert!(matches!(
            ask_parsed(&mut io, "端口", Some("443"), |s| s
                .parse::<u16>()
                .map_err(|e| e.to_string())),
            Err(PromptError::Cancelled)
        ));

        let mut io = BufferedIo::new(io::Cursor::new("0\n"), Vec::new());
        assert!(matches!(
            io.confirm("确认执行?", false),
            Err(PromptError::Cancelled)
        ));
    }

    #[test]
    fn invalid_input_retries_then_accepts_zero_cancel() {
        let mut io = BufferedIo::new(io::Cursor::new("bad\n0\n"), Vec::new());
        let res = ask_parsed(&mut io, "端口", None, |s| {
            s.parse::<u16>().map_err(|_| "必须是整数".into())
        });
        assert!(matches!(res, Err(PromptError::Cancelled)));
        let printed = String::from_utf8(io.writer).unwrap();
        assert!(printed.contains("必须是整数"));
    }

    #[test]
    fn menu_choice_maps_zero_and_empty_to_zero() {
        let mut io = BufferedIo::new(io::Cursor::new("\n0\n3\n"), Vec::new());
        assert_eq!(ask_menu_choice(&mut io, "选择", 8).unwrap(), 0);
        assert_eq!(ask_menu_choice(&mut io, "选择", 8).unwrap(), 0);
        assert_eq!(ask_menu_choice(&mut io, "选择", 8).unwrap(), 3);
    }
}
