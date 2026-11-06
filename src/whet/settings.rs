//! `<home>/settings.toml`: what whet installed and where, so it can update and uninstall cleanly.

use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub default_toolchain: Option<String>,
    pub dist_server: Option<String>,
    pub profile: String,
    /// Shell profile files that received the PATH block.
    pub touched: Vec<String>,
    /// Files whet created outside its home (fish config, completions).
    pub created: Vec<String>,
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_strings(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut escape = false;
    for c in s.chars() {
        if in_str {
            if escape {
                cur.push(c);
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_str = true;
        }
    }
    out
}

pub fn parse(text: &str) -> Settings {
    let mut s = Settings { profile: "default".to_string(), ..Default::default() };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let strs = parse_strings(v);
        match k {
            "default_toolchain" => s.default_toolchain = strs.into_iter().next(),
            "dist_server" => s.dist_server = strs.into_iter().next(),
            "profile" => {
                if let Some(p) = strs.into_iter().next() {
                    s.profile = p;
                }
            }
            "touched" => s.touched = strs,
            "created" => s.created = strs,
            _ => {}
        }
    }
    s
}

pub fn render(s: &Settings) -> String {
    let list = |v: &[String]| format!("[{}]", v.iter().map(|x| format!("\"{}\"", esc(x))).collect::<Vec<_>>().join(", "));
    let mut out = String::from("# managed by whet; edit with care\nversion = 1\n");
    if let Some(t) = &s.default_toolchain {
        out.push_str(&format!("default_toolchain = \"{}\"\n", esc(t)));
    }
    if let Some(d) = &s.dist_server {
        out.push_str(&format!("dist_server = \"{}\"\n", esc(d)));
    }
    out.push_str(&format!("profile = \"{}\"\n", esc(&s.profile)));
    out.push_str(&format!("touched = {}\n", list(&s.touched)));
    out.push_str(&format!("created = {}\n", list(&s.created)));
    out
}

pub fn exists(home: &Path) -> bool {
    home.join("settings.toml").is_file()
}

pub fn load(home: &Path) -> Settings {
    match std::fs::read_to_string(home.join("settings.toml")) {
        Ok(t) => parse(&t),
        Err(_) => Settings { profile: "default".to_string(), ..Default::default() },
    }
}

pub fn save(home: &Path, s: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("cannot create `{}`: {}", home.display(), e))?;
    let p = home.join("settings.toml");
    std::fs::write(&p, render(s)).map_err(|e| format!("cannot write `{}`: {}", p.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_including_awkward_paths() {
        let s = Settings {
            default_toolchain: Some("0.2.1".into()),
            dist_server: Some("https://example.org/dist".into()),
            profile: "minimal".into(),
            touched: vec!["/home/a b/.profile".into(), "/home/\"q\"/.bashrc".into()],
            created: vec!["/x/y\\z".into()],
        };
        assert_eq!(parse(&render(&s)), s);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let s = load(Path::new("/definitely/not/here"));
        assert_eq!(s.profile, "default");
        assert!(s.default_toolchain.is_none());
    }
}
