//! Diagnostics shared by every phase.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether diagnostics use ANSI colors (set once by the CLI).
pub static COLOR: AtomicBool = AtomicBool::new(false);

fn paint(s: &str, code: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub code: &'static str,
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

pub type R<T> = Result<T, Diag>;

impl Diag {
    pub fn new(code: &'static str, msg: impl Into<String>, line: u32, col: u32) -> Diag {
        Diag { code, msg: msg.into(), line, col }
    }

    /// `file:line:col: error[E0101]: message` (one line, no source snippet).
    pub fn render(&self, file: &str) -> String {
        if self.col > 0 {
            format!("{}:{}:{}: error[{}]: {}", file, self.line, self.col, self.code, self.msg)
        } else if self.line > 0 {
            format!("{}:{}: error[{}]: {}", file, self.line, self.code, self.msg)
        } else {
            format!("{}: error[{}]: {}", file, self.code, self.msg)
        }
    }

    /// A rustc-style diagnostic with a source snippet:
    ///
    /// ```text
    /// error[E0201]: comparison operators cannot be chained; use `&&`
    ///  --> demo.kn:3:11
    ///   |
    /// 3 |     a < 1 < 2
    ///   |           ^
    ///   = help: run `keen explain E0201` for details
    /// ```
    pub fn render_with_source(&self, file: &str, src: &str) -> String {
        let mut out = format!(
            "{}: {}\n",
            paint(&format!("error[{}]", self.code), "1;31"),
            paint(&self.msg, "1")
        );
        if self.line == 0 {
            out.push_str(&format!("{} {}\n", paint("-->", "1;34"), file));
            out.push_str(&self.help());
            return out;
        }
        let ln = self.line as usize;
        let w = ln.to_string().len();
        let pad = " ".repeat(w);
        let loc = if self.col > 0 { format!("{}:{}:{}", file, ln, self.col) } else { format!("{}:{}", file, ln) };
        out.push_str(&format!("{}{} {}\n", pad, paint("-->", "1;34"), loc));
        if let Some(text) = src.lines().nth(ln - 1) {
            let bar = paint("|", "1;34");
            out.push_str(&format!("{} {}\n", pad, bar));
            out.push_str(&format!("{} {} {}\n", paint(&format!("{:>w$}", ln, w = w), "1;34"), bar, text.replace('\t', "    ")));
            if self.col > 0 {
                let prefix: String = text.chars().take(self.col as usize - 1).collect();
                let offset: usize = prefix.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
                out.push_str(&format!("{} {} {}{}\n", pad, bar, " ".repeat(offset), paint("^", "1;31")));
            }
        }
        out.push_str(&self.help());
        out
    }

    fn help(&self) -> String {
        format!(
            "  {} {}\n",
            paint("= help:", "1;36"),
            format!("run `keen explain {}` for details", self.code)
        )
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.msg)
    }
}

impl std::error::Error for Diag {}

/// Shorthand used by the emitter (no column information is available there).
pub fn fail<T>(code: &'static str, msg: impl Into<String>, line: u32) -> R<T> {
    Err(Diag::new(code, msg, line, 0))
}
