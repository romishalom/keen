//! `keen.toml`: a deliberately tiny TOML subset (sections, strings, integers, booleans and
//! arrays of strings) so keen stays dependency-free.
//!
//! ```toml
//! [package]
//! name = "hello"
//! version = "0.1.0"
//! entry = "src/main.kn"      # optional, default src/main.kn
//!
//! [build]
//! std = "c11"                # "c11" or "c23"
//! cc = "clang"               # optional; default: $KEEN_CC, $CC, cc, gcc, clang
//! opt = "2"                  # optional optimization level for every profile
//! warnings = true
//! cflags = ["-DFEATURE_X"]
//! ldflags = ["-lm"]
//! sources = ["c/helper.c"]   # extra C files to compile and link
//! ```

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: String,
    pub std: Option<String>,
    pub cc: Option<String>,
    pub opt: Option<String>,
    pub warnings: Option<bool>,
    pub cflags: Vec<String>,
    pub ldflags: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '#' {
            return &line[..i];
        }
    }
    line
}

fn parse_string(s: &str) -> Result<(String, &str), String> {
    // s starts with a quote
    let mut out = String::new();
    let mut chars = s.char_indices().skip(1);
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((out, &s[i + 1..])),
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, o)) => return Err(format!("unsupported escape `\\{}`", o)),
                None => break,
            },
            _ => out.push(c),
        }
    }
    Err("unterminated string".to_string())
}

fn parse_value(v: &str) -> Result<Value, String> {
    let v = v.trim();
    if v.starts_with('"') {
        let (s, rest) = parse_string(v)?;
        if !rest.trim().is_empty() {
            return Err(format!("unexpected text after string: `{}`", rest.trim()));
        }
        return Ok(Value::Str(s));
    }
    if v == "true" || v == "false" {
        return Ok(Value::Bool(v == "true"));
    }
    if let Some(inner) = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        let mut items = Vec::new();
        let mut rest = inner.trim();
        while !rest.is_empty() {
            if !rest.starts_with('"') {
                return Err("arrays may only contain strings".to_string());
            }
            let (s, r) = parse_string(rest)?;
            items.push(s);
            rest = r.trim_start();
            if let Some(r) = rest.strip_prefix(',') {
                rest = r.trim_start();
            } else if !rest.is_empty() {
                return Err("expected `,` between array items".to_string());
            }
        }
        return Ok(Value::List(items));
    }
    v.parse::<i64>().map(Value::Int).map_err(|_| format!("cannot parse value `{}`", v))
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let mut section = String::new();
    let mut m = Manifest {
        name: String::new(),
        version: "0.1.0".to_string(),
        entry: "src/main.kn".to_string(),
        std: None,
        cc: None,
        opt: None,
        warnings: None,
        cflags: Vec::new(),
        ldflags: Vec::new(),
        sources: Vec::new(),
    };
    let mut lines = text.lines().enumerate();
    while let Some((n, raw)) = lines.next() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        let at = |msg: String| format!("keen.toml:{}: {}", n + 1, msg);
        if line.starts_with('[') && line.ends_with(']') && !line.contains('=') {
            section = line[1..line.len() - 1].trim().to_string();
            if section != "package" && section != "build" {
                return Err(at(format!("unknown section `[{}]` (expected [package] or [build])", section)));
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(at(format!("expected `key = value`, found `{}`", line)));
        };
        let key = k.trim().to_string();
        let mut vtext = v.trim().to_string();
        // multi-line arrays
        while vtext.starts_with('[') && !vtext.ends_with(']') {
            match lines.next() {
                Some((_, more)) => {
                    vtext.push(' ');
                    vtext.push_str(strip_comment(more).trim());
                }
                None => return Err(at("unterminated array".to_string())),
            }
        }
        let val = parse_value(&vtext).map_err(&at)?;
        let want_str = |v: Value| -> Result<String, String> {
            match v {
                Value::Str(s) => Ok(s),
                _ => Err(at(format!("`{}` must be a string", key))),
            }
        };
        let want_list = |v: Value| -> Result<Vec<String>, String> {
            match v {
                Value::List(l) => Ok(l),
                _ => Err(at(format!("`{}` must be an array of strings", key))),
            }
        };
        match (section.as_str(), key.as_str()) {
            ("package", "name") => m.name = want_str(val)?,
            ("package", "version") => m.version = want_str(val)?,
            ("package", "entry") => m.entry = want_str(val)?,
            ("build", "std") => {
                let s = want_str(val)?;
                if s != "c11" && s != "c23" {
                    return Err(at("`std` must be \"c11\" or \"c23\"".to_string()));
                }
                m.std = Some(s);
            }
            ("build", "cc") => m.cc = Some(want_str(val)?),
            ("build", "opt") => {
                m.opt = Some(match val {
                    Value::Str(s) => s,
                    Value::Int(i) => i.to_string(),
                    _ => return Err(at("`opt` must be a string or integer".to_string())),
                })
            }
            ("build", "warnings") => match val {
                Value::Bool(b) => m.warnings = Some(b),
                _ => return Err(at("`warnings` must be true or false".to_string())),
            },
            ("build", "cflags") => m.cflags = want_list(val)?,
            ("build", "ldflags") => m.ldflags = want_list(val)?,
            ("build", "sources") => m.sources = want_list(val)?,
            (sec, key) => {
                let where_ = if sec.is_empty() { "at the top level".to_string() } else { format!("in [{}]", sec) };
                return Err(at(format!("unknown key `{}` {}", key, where_)));
            }
        }
    }
    if m.name.is_empty() {
        return Err("keen.toml: missing `name` in [package]".to_string());
    }
    let ok = m.name.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
        && m.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        return Err(format!(
            "keen.toml: package name `{}` must start with a letter or `_` and contain only letters, digits, `_` and `-`",
            m.name
        ));
    }
    Ok(m)
}

/// Walk up from `start` looking for `keen.toml`; returns the directory that contains it.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut dir = std::fs::canonicalize(start).ok()?;
    loop {
        if dir.join("keen.toml").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn load(root: &Path) -> Result<Manifest, String> {
    let p = root.join("keen.toml");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("cannot read `{}`: {}", p.display(), e))?;
    parse(&text)
}

pub fn template(name: &str) -> String {
    format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[build]\nstd = \"c11\"\n# cc = \"clang\"\n# opt = \"2\"\n# cflags = [\"-DFEATURE_X\"]\n# ldflags = [\"-lm\"]\n# sources = [\"c/helper.c\"]\n",
        name
    )
}

pub fn main_template(name: &str) -> String {
    format!(
        "use <assert.h>;\nuse <stdio.h>;\nuse <string.h>;\n\nfn greeting() -> *const c_char {{\n    \"Hello from {}!\"\n}}\n\npub fn main() -> i32 {{\n    printf(\"%s\\n\", greeting());\n    return 0;\n}}\n\n#[test]\nfn greeting_is_not_empty() {{\n    assert(strlen(greeting()) > 0);\n}}\n",
        name
    )
}
