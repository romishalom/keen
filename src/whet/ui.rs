//! Terminal output and prompts (messages go to stderr, menus and prompts to stdout).

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static COLOR: AtomicBool = AtomicBool::new(false);

pub fn setup(quiet: bool, verbose: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
    VERBOSE.store(verbose, Ordering::Relaxed);
    let color = io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |t| t != "dumb");
    COLOR.store(color, Ordering::Relaxed);
}

pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn paint(s: &str, code: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

pub fn info(msg: &str) {
    if !quiet() {
        eprintln!("{} {}", paint("info:", "1;32"), msg);
    }
}

pub fn debug(msg: &str) {
    if verbose() {
        eprintln!("{} {}", paint("debug:", "2"), msg);
    }
}

pub fn warn(msg: &str) {
    eprintln!("{} {}", paint("warn:", "1;33"), msg);
}

pub fn error(msg: &str) {
    eprintln!("{} {}", paint("error:", "1;31"), msg);
}

pub fn bold(s: &str) -> String {
    paint(s, "1")
}

/// Prompts need a terminal (or `WHET_ALLOW_PIPED_PROMPTS=1`, used by tests).
pub fn can_prompt() -> bool {
    io::stdin().is_terminal() || std::env::var_os("WHET_ALLOW_PIPED_PROMPTS").is_some()
}

/// Read one line from stdin; `None` on end of input.
pub fn read_line() -> Option<String> {
    let mut s = String::new();
    match io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim().to_string()),
    }
}

pub fn ask(prompt: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{}: ", prompt);
    } else {
        print!("{} [{}]: ", prompt, default);
    }
    let _ = io::stdout().flush();
    match read_line() {
        Some(s) if !s.is_empty() => s,
        _ => default.to_string(),
    }
}

/// A bare prompt (`> `) whose default is described by the surrounding text.
pub fn ask_plain(prompt: &str, default: &str) -> String {
    print!("{} ", prompt);
    let _ = io::stdout().flush();
    match read_line() {
        Some(s) if !s.is_empty() => s,
        _ => default.to_string(),
    }
}

pub fn ask_yes_no(prompt: &str, default: bool) -> bool {
    loop {
        print!("{} [{}]: ", prompt, if default { "Y/n" } else { "y/N" });
        let _ = io::stdout().flush();
        match read_line().map(|s| s.to_lowercase()) {
            None => return default,
            Some(s) if s.is_empty() => return default,
            Some(s) if s == "y" || s == "yes" => return true,
            Some(s) if s == "n" || s == "no" => return false,
            _ => println!("please answer y or n"),
        }
    }
}
