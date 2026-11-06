//! Downloads. whet shells out to `curl` (or `wget`) instead of linking an HTTP stack, and handles
//! `file://` URLs itself, which makes offline installs and testing trivial.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_loopback(url: &str) -> bool {
    let rest = url.trim_start_matches("http://");
    let host = rest.split(['/', ':']).next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1" || host == "[::1]"
}

/// Reject downgrade-prone URLs: only https, file://, or http to the local machine.
pub fn check_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("file://") {
        return Ok(());
    }
    if url.starts_with("http://") {
        if is_loopback(url) || std::env::var_os("WHET_ALLOW_HTTP").is_some() {
            return Ok(());
        }
        return Err(format!(
            "refusing to download over plain http: {} (use https, or set WHET_ALLOW_HTTP=1 if you really mean it)",
            url
        ));
    }
    Err(format!("unsupported URL `{}` (expected https://, file:// or a loopback http:// URL)", url))
}

pub fn fetch(url: &str, dest: &Path, progress: bool) -> Result<(), String> {
    check_url(url)?;
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::copy(path, dest)
            .map(|_| ())
            .map_err(|e| format!("cannot read `{}`: {}", path, e));
    }
    let status = if have("curl") {
        let mut c = Command::new("curl");
        c.args(["--fail", "--location", "--retry", "3", "--connect-timeout", "20", "--proto-redir", "=https,http"]);
        c.arg(if progress { "--progress-bar" } else { "--silent" });
        c.arg("--show-error").arg("--output").arg(dest).arg(url);
        c.status()
    } else if have("wget") {
        let mut c = Command::new("wget");
        c.args(["--tries=3", "--quiet"]);
        if progress {
            c.arg("--show-progress");
        }
        c.arg("-O").arg(dest).arg(url);
        c.status()
    } else {
        return Err("neither `curl` nor `wget` was found; install one of them (or use a file:// dist server)".into());
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => {
            let _ = std::fs::remove_file(dest);
            Err(format!("download failed: {}", url))
        }
        Err(e) => Err(format!("cannot start the downloader: {}", e)),
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("whet-{}-{}-{}", tag, std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)))
}

pub fn fetch_text(url: &str) -> Result<String, String> {
    let tmp = temp_path("text");
    let r = fetch(url, &tmp, false).and_then(|_| {
        std::fs::read_to_string(&tmp).map_err(|e| format!("cannot read downloaded file: {}", e))
    });
    let _ = std::fs::remove_file(&tmp);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        assert!(check_url("https://example.org/x").is_ok());
        assert!(check_url("file:///tmp/dist/latest").is_ok());
        assert!(check_url("http://127.0.0.1:8000/latest").is_ok());
        assert!(check_url("http://localhost/latest").is_ok());
        assert!(check_url("http://example.org/latest").is_err());
        assert!(check_url("ftp://example.org/x").is_err());
        assert!(check_url("example.org/x").is_err());
    }

    #[test]
    fn file_urls_are_copied() {
        let src = temp_path("src");
        std::fs::write(&src, "0.9.0\n").unwrap();
        let text = fetch_text(&format!("file://{}", src.display())).unwrap();
        assert_eq!(text, "0.9.0\n");
        let _ = std::fs::remove_file(src);
        assert!(fetch_text("file:///no/such/file").is_err());
    }
}
