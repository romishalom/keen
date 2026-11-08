//! Toolchains: `<home>/toolchains/<version>/bin/keen`, and the `<home>/bin/keen` link to the default.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::net;
use crate::platform::{self, Version};
use crate::sha256;
use crate::ui;

pub struct Ctx {
    pub home: PathBuf,
    pub dist: String,
    pub target: String,
}

impl Ctx {
    pub fn bin_dir(&self) -> PathBuf {
        self.home.join("bin")
    }
    pub fn toolchains_dir(&self) -> PathBuf {
        self.home.join("toolchains")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.home.join("tmp")
    }
    pub fn toolchain_dir(&self, v: &str) -> PathBuf {
        self.toolchains_dir().join(v)
    }
    pub fn exe(name: &str) -> String {
        format!("{}{}", name, std::env::consts::EXE_SUFFIX)
    }
}

/// `latest` asks the dist server; anything else must look like a version.
pub fn resolve(ctx: &Ctx, spec: &str) -> Result<String, String> {
    if spec == "latest" || spec == "stable" {
        // GitHub release download URL for the 'latest' asset file
        let url = format!("{}/latest/download/latest", ctx.dist.trim_end_matches('/'));
        let text = net::fetch_text(&url)
            .map_err(|e| format!("cannot determine the latest version: {}", e))?;
        let v = text.lines().next().unwrap_or("").trim().to_string();
        if !platform::valid_version(&v) {
            return Err(format!("the release returned an invalid version `{}`", v));
        }
        return Ok(v);
    }
    let v = spec.trim_start_matches('v');
    if platform::valid_version(v) {
        Ok(v.to_string())
    } else {
        Err(format!("`{}` is not a version (expected `latest` or something like 0.2.1)", spec))
    }
}

/// Installed toolchain versions, oldest first.
pub fn list(ctx: &Ctx) -> Vec<String> {
    let mut v: Vec<Version> = fs::read_dir(ctx.toolchains_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().join("bin").join(Ctx::exe("keen")).exists())
                .filter_map(|e| Version::parse(&e.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.into_iter()
        .map(|v| {
            let mut s = format!("{}.{}.{}", v.nums[0], v.nums[1], v.nums[2]);
            if let Some(p) = v.pre {
                s.push('-');
                s.push_str(&p);
            }
            s
        })
        .collect()
}

fn unsafe_entry(line: &str) -> bool {
    let l = line.trim();
    l.starts_with('/') || l.starts_with('\\') || l.split(['/', '\\']).any(|c| c == "..") || l.contains(':') && l.len() > 1 && l.as_bytes()[1] == b':'
}

/// Download, verify and unpack a toolchain. Returns `false` if it was already installed.
pub fn install(ctx: &Ctx, version: &str, force: bool) -> Result<bool, String> {
    let dest = ctx.toolchain_dir(version);
    if dest.join("bin").join(Ctx::exe("keen")).exists() && !force {
        return Ok(false);
    }
    if !net::have("tar") {
        return Err("`tar` is required to unpack toolchains but was not found".into());
    }
    let dist = ctx.dist.trim_end_matches('/');
    let name = format!("keen-{}-{}", version, ctx.target);
    
     // GitHub releases URL format: /releases/download/v<version>/<filename>
    let url = format!("{}/download/v{}/{}.tar.gz", dist, version, name);
    let work = ctx.tmp_dir().join(format!("install-{}-{}", version, std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("cannot create `{}`: {}", work.display(), e))?;
    let result = (|| -> Result<(), String> {
        let archive = work.join(format!("{}.tar.gz", name));
        ui::info(&format!("downloading keen {} ({})", version, ctx.target));
        net::fetch(&url, &archive, !ui::quiet() && std::io::IsTerminal::is_terminal(&std::io::stderr())).map_err(|e| {
            format!(
                "{}\n       (is {} published for target {}? check `whet toolchain list` and GitHub releases)",
                e, version, ctx.target
            )
        })?;
        ui::info("verifying checksum");
        verify(&archive, &format!("{}.sha256", url))?;
        // refuse archives that try to escape the extraction directory
        let listing = Command::new("tar")
            .arg("-tzf")
            .arg(&archive)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run tar: {}", e))?;
        if !listing.status.success() {
            return Err("the downloaded archive is not a valid .tar.gz".into());
        }
        for l in String::from_utf8_lossy(&listing.stdout).lines() {
            if unsafe_entry(l) {
                return Err(format!("the archive contains an unsafe path `{}`; refusing to unpack it", l));
            }
        }
        ui::info(&format!("installing toolchain {}", version));
        let st = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&work)
            .status()
            .map_err(|e| format!("cannot run tar: {}", e))?;
        if !st.success() {
            return Err("tar failed to unpack the archive".into());
        }
        let root = work.join(&name);
        if !root.join("bin").join(Ctx::exe("keen")).exists() {
            return Err(format!("the archive does not contain {}/bin/{}", name, Ctx::exe("keen")));
        }
        fs::create_dir_all(ctx.toolchains_dir()).map_err(|e| e.to_string())?;
        if dest.exists() {
            fs::remove_dir_all(&dest).map_err(|e| format!("cannot replace `{}`: {}", dest.display(), e))?;
        }
        fs::rename(&root, &dest).map_err(|e| format!("cannot move the toolchain into place: {}", e))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&work);
    result.map(|_| true)
}

/// Compare a file against the hash in `<url>` (`<hex>  <name>`).
pub fn verify(file: &Path, sha_url: &str) -> Result<(), String> {
    let text = net::fetch_text(sha_url).map_err(|e| format!("cannot fetch the checksum: {}", e))?;
    let want = text.split_whitespace().next().unwrap_or("").to_lowercase();
    if want.len() != 64 || !want.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("malformed checksum file at {}", sha_url));
    }
    let got = sha256::file_hex(file).map_err(|e| format!("cannot hash `{}`: {}", file.display(), e))?;
    if got != want {
        return Err(format!(
            "checksum mismatch for {}\n       expected {}\n       got      {}\n       (the download is corrupt or has been tampered with; nothing was installed)",
            file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            want,
            got
        ));
    }
    Ok(())
}

pub fn uninstall(ctx: &Ctx, version: &str, default: Option<&str>) -> Result<(), String> {
    if default == Some(version) {
        return Err(format!("{} is the default toolchain; run `whet default <other>` first", version));
    }
    let dir = ctx.toolchain_dir(version);
    if !dir.exists() {
        return Err(format!("toolchain {} is not installed", version));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove `{}`: {}", dir.display(), e))
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(not(unix))]
fn link(from: &Path, to: &Path) -> std::io::Result<()> {
    // no symlinks without privileges on Windows: copy instead
    let src = to.parent().unwrap_or(Path::new(".")).join(from);
    fs::copy(src, to).map(|_| ())
}

/// Point `<home>/bin/keen` at the given toolchain.
pub fn set_default(ctx: &Ctx, version: &str) -> Result<(), String> {
    let real = ctx.toolchain_dir(version).join("bin").join(Ctx::exe("keen"));
    if !real.exists() {
        return Err(format!("toolchain {} is not installed (try `whet install {}`)", version, version));
    }
    fs::create_dir_all(ctx.bin_dir()).map_err(|e| format!("cannot create `{}`: {}", ctx.bin_dir().display(), e))?;
    let dest = ctx.bin_dir().join(Ctx::exe("keen"));
    let _ = fs::remove_file(&dest);
    let rel = PathBuf::from("..").join("toolchains").join(version).join("bin").join(Ctx::exe("keen"));
    link(&rel, &dest).map_err(|e| format!("cannot link `{}`: {}", dest.display(), e))
}

pub fn make_executable(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = fs::metadata(p) {
            let mut perm = md.permissions();
            perm.set_mode(0o755);
            let _ = fs::set_permissions(p, perm);
        }
    }
    #[cfg(not(unix))]
    let _ = p;
}

/// Is a C compiler available? keen needs one to build programs.
pub fn find_cc() -> Option<String> {
    for cand in ["cc", "gcc", "clang"] {
        if net::have(cand) {
            return Some(cand.to_string());
        }
    }
    None
}

pub fn cc_install_hint() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS: run `xcode-select --install`",
        "windows" => "Windows: install MSYS2/MinGW-w64 gcc or LLVM clang and put it on PATH",
        _ => "Debian/Ubuntu: `sudo apt install build-essential`; Fedora: `sudo dnf install gcc`; Arch: `sudo pacman -S gcc`; Alpine: `apk add build-base`",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_archive_paths_are_detected() {
        assert!(unsafe_entry("/etc/passwd"));
        assert!(unsafe_entry("keen-1.0.0/../../x"));
        assert!(unsafe_entry("..\\x"));
        assert!(unsafe_entry("C:/x"));
        assert!(!unsafe_entry("keen-1.0.0/bin/keen"));
        assert!(!unsafe_entry("keen-1.0.0/share/a..b"));
    }

    #[test]
    fn resolve_validates_versions() {
        let ctx = Ctx { home: PathBuf::from("/nope"), dist: "file:///nope".into(), target: "t".into() };
        assert_eq!(resolve(&ctx, "0.2.1").unwrap(), "0.2.1");
        assert_eq!(resolve(&ctx, "v1.0.0").unwrap(), "1.0.0");
        assert!(resolve(&ctx, "../../x").is_err());
        assert!(resolve(&ctx, "latest").is_err()); // dist unreachable
    }
}
