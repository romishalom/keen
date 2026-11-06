//! End-to-end tests for `whet` and `scripts/install.sh` against a fixture release server
//! (a plain directory used through file:// URLs). Needs `tar` and sha256sum/shasum; skipped without.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const TARGET: &str = "test-target-linux";

fn whet_bin() -> &'static str {
    env!("CARGO_BIN_EXE_whet")
}

fn have(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn tools() -> bool {
    let ok = have("tar") && (have("sha256sum") || have("shasum"));
    if !ok {
        eprintln!("tar/sha256sum not available; skipping");
    }
    ok
}

struct Env {
    root: PathBuf,
    home: PathBuf,
    keen_home: PathBuf,
    dist: PathBuf,
}

fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("whet-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, dist) = (root.join("home"), root.join("dist"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&dist).unwrap();
    Env { keen_home: home.join(".keen"), root, home, dist }
}

fn sha_file(path: &Path) {
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let out = if have("sha256sum") {
        Command::new("sha256sum").arg(&name).current_dir(dir).output().unwrap()
    } else {
        Command::new("shasum").args(["-a", "256", &name]).current_dir(dir).output().unwrap()
    };
    std::fs::write(dir.join(format!("{}.sha256", name)), out.stdout).unwrap();
}

/// Publish `ver` into the fixture server: a fake `keen`, and a copy of the real `whet`.
fn add_release(e: &Env, ver: &str, set_latest: bool) {
    let name = format!("keen-{}-{}", ver, TARGET);
    let stage = e.root.join(format!("stage-{}", ver));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(stage.join(&name).join("bin")).unwrap();
    let keen = stage.join(&name).join("bin/keen");
    std::fs::write(
        &keen,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo \"keen {v}\" ;;\n  completions) echo \"# keen completions for $2\" ;;\n  *) echo \"keen {v} ran: $*\" ;;\nesac\n",
            v = ver
        ),
    )
    .unwrap();
    std::fs::set_permissions(&keen, std::fs::Permissions::from_mode(0o755)).unwrap();
    let kdir = e.dist.join("keen").join(ver);
    std::fs::create_dir_all(&kdir).unwrap();
    let archive = kdir.join(format!("{}.tar.gz", name));
    assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&stage).arg(&name).status().unwrap().success());
    sha_file(&archive);
    let wdir = e.dist.join("whet").join(ver);
    std::fs::create_dir_all(&wdir).unwrap();
    let wfile = wdir.join(format!("whet-{}", TARGET));
    std::fs::copy(whet_bin(), &wfile).unwrap();
    sha_file(&wfile);
    if set_latest {
        std::fs::write(e.dist.join("latest"), format!("{}\n", ver)).unwrap();
    }
}

fn whet(e: &Env) -> Command {
    let mut c = Command::new(whet_bin());
    c.env("HOME", &e.home)
        .env("KEEN_HOME", &e.keen_home)
        .env("WHET_DIST_SERVER", format!("file://{}", e.dist.display()))
        .env("WHET_TARGET", TARGET)
        .env("SHELL", "/bin/bash")
        .env_remove("ZDOTDIR")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("WHET_ALLOW_PIPED_PROMPTS")
        .stdin(Stdio::null());
    c
}

fn run(c: &mut Command) -> Output {
    c.output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn keen_version(e: &Env) -> String {
    out(&Command::new(e.keen_home.join("bin/keen")).arg("--version").output().unwrap()).trim().to_string()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

const BEGIN: &str = "# >>> keen (whet) >>>";

// ------------------------------------------------------------------------ init

#[test]
fn init_yes_installs_the_latest_toolchain() {
    if !tools() {
        return;
    }
    let e = env("init");
    add_release(&e, "0.2.0", false);
    add_release(&e, "0.2.1", true);
    let o = run(whet(&e).args(["init", "-y", "-q"]));
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("keen 0.2.1 is installed now"), "{}", out(&o));

    let link = e.keen_home.join("bin/keen");
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(keen_version(&e), "keen 0.2.1");
    assert!(e.keen_home.join("bin/whet").is_file());
    assert!(e.keen_home.join("env").is_file());
    assert!(read(&e.home.join(".profile")).contains(BEGIN));
    assert!(read(&e.home.join(".bashrc")).contains(". \"$HOME/.keen/env\""));
    assert_eq!(read(&e.keen_home.join("share/completions/keen.bash")).trim(), "# keen completions for bash");
    let settings = read(&e.keen_home.join("settings.toml"));
    assert!(settings.contains("default_toolchain = \"0.2.1\""), "{}", settings);
    assert!(!e.keen_home.join("toolchains/0.2.0").exists(), "only the requested toolchain is installed");
    assert!(std::fs::read_dir(e.keen_home.join("tmp")).unwrap().next().is_none(), "temp files cleaned up");
}

#[test]
fn init_is_idempotent() {
    if !tools() {
        return;
    }
    let e = env("idem");
    add_release(&e, "0.2.1", true);
    for _ in 0..3 {
        let o = run(whet(&e).args(["init", "-y", "-q", "--force"]));
        assert!(o.status.success(), "{}", err(&o));
    }
    assert_eq!(read(&e.home.join(".profile")).matches(BEGIN).count(), 1);
    assert_eq!(read(&e.home.join(".bashrc")).matches(BEGIN).count(), 1);
    assert_eq!(keen_version(&e), "keen 0.2.1");
}

#[test]
fn existing_profile_content_is_preserved() {
    if !tools() {
        return;
    }
    let e = env("preserve");
    add_release(&e, "0.2.1", true);
    std::fs::write(e.home.join(".bashrc"), "export A=1\nalias x=y").unwrap();
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());
    let text = read(&e.home.join(".bashrc"));
    assert!(text.starts_with("export A=1\nalias x=y\n"), "{}", text);
    assert!(text.contains(BEGIN));
}

#[test]
fn advanced_options_are_honoured() {
    if !tools() {
        return;
    }
    let e = env("advanced");
    add_release(&e, "0.1.0", false);
    add_release(&e, "0.2.1", true);
    let prefix = e.root.join("opt/keen");
    let o = run(whet(&e).args(["init", "-y", "-q", "--default-toolchain", "0.1.0", "--profile", "minimal"]).arg("--no-modify-path").arg("--prefix").arg(&prefix));
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&Command::new(prefix.join("bin/keen")).arg("--version").output().unwrap()).trim(), "keen 0.1.0");
    assert!(!e.home.join(".profile").exists() && !e.home.join(".bashrc").exists(), "--no-modify-path leaves profiles alone");
    assert!(prefix.join("env").is_file(), "the env file is still written");
    assert!(!prefix.join("share/completions/keen.bash").exists(), "minimal profile has no completions");
    assert!(!e.keen_home.exists(), "nothing at the default location");
}

#[test]
fn prefix_outside_home_uses_an_absolute_path_in_profiles() {
    if !tools() {
        return;
    }
    let e = env("abs");
    add_release(&e, "0.2.1", true);
    let prefix = e.root.join("opt/keen");
    assert!(run(whet(&e).args(["init", "-y", "-q", "--prefix"]).arg(&prefix)).status.success());
    let text = read(&e.home.join(".profile"));
    assert!(text.contains(&format!(". \"{}/env\"", prefix.display())), "{}", text);
}

#[test]
fn whet_only_install_with_toolchain_none() {
    if !tools() {
        return;
    }
    let e = env("none");
    add_release(&e, "0.2.1", true);
    let o = run(whet(&e).args(["init", "-y", "-q", "--default-toolchain", "none"]));
    assert!(o.status.success(), "{}", err(&o));
    assert!(e.keen_home.join("bin/whet").is_file());
    assert!(!e.keen_home.join("bin/keen").exists());
    let o = run(whet(&e).args(["install", "latest"]));
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("now the default"), "{}", out(&o));
    assert_eq!(keen_version(&e), "keen 0.2.1");
}

#[test]
fn bad_checksum_installs_nothing() {
    if !tools() {
        return;
    }
    let e = env("sum");
    add_release(&e, "0.2.1", true);
    let archive = e.dist.join("keen/0.2.1").join(format!("keen-0.2.1-{}.tar.gz", TARGET));
    let mut bytes = std::fs::read(&archive).unwrap();
    bytes.push(0);
    std::fs::write(&archive, bytes).unwrap();
    let o = run(whet(&e).args(["init", "-y", "-q"]));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("checksum mismatch"), "{}", err(&o));
    assert!(!e.keen_home.join("bin/keen").exists());
    assert!(!e.keen_home.join("toolchains/0.2.1").exists());
    assert!(!e.home.join(".profile").exists(), "profiles untouched after a failed install");
}

#[test]
fn unknown_and_invalid_versions_are_rejected() {
    if !tools() {
        return;
    }
    let e = env("badver");
    add_release(&e, "0.2.1", true);
    let o = run(whet(&e).args(["init", "-y", "-q", "--default-toolchain", "9.9.9"]));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("download failed") || err(&o).contains("cannot read"), "{}", err(&o));
    assert!(!e.home.join(".profile").exists());
    let o = run(whet(&e).args(["init", "-y", "--default-toolchain", "banana"]));
    assert_eq!(o.status.code(), Some(2));
    let o = run(whet(&e).args(["init", "-y", "--default-toolchain", "../../x"]));
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn plain_http_to_a_remote_host_is_refused() {
    let e = env("http");
    let o = run(whet(&e).args(["init", "-y", "--dist-server", "http://example.org/dist"]));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("plain http"), "{}", err(&o));
}

#[test]
fn prompts_need_a_terminal_unless_yes() {
    let e = env("noterm");
    let o = run(whet(&e).arg("init"));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("-y"), "{}", err(&o));
}

// -------------------------------------------------------------- interactive menu

fn with_input(e: &Env, args: &[&str], input: &str) -> Output {
    use std::io::Write;
    let mut child = whet(e)
        .args(args)
        .env("WHET_ALLOW_PIPED_PROMPTS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn menu_enter_accepts_the_standard_installation() {
    if !tools() {
        return;
    }
    let e = env("menu1");
    add_release(&e, "0.2.1", true);
    let o = with_input(&e, &["init", "-q"], "\n");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    let text = out(&o);
    assert!(text.contains("Welcome to keen!") && text.contains("Proceed with standard installation"), "{}", text);
    assert!(text.contains("Customize installation") && text.contains("Cancel installation"), "{}", text);
    assert_eq!(keen_version(&e), "keen 0.2.1");
}

#[test]
fn menu_cancel_installs_nothing() {
    if !tools() {
        return;
    }
    let e = env("menu3");
    add_release(&e, "0.2.1", true);
    let o = with_input(&e, &["init", "-q"], "3\n");
    assert!(o.status.success());
    assert!(out(&o).contains("cancelled"), "{}", out(&o));
    assert!(!e.keen_home.exists() && !e.home.join(".profile").exists());
}

#[test]
fn menu_customize_then_proceed() {
    if !tools() {
        return;
    }
    let e = env("menu2");
    add_release(&e, "0.1.0", false);
    add_release(&e, "0.2.1", true);
    // choose 2, then: toolchain, profile, modify PATH?, install dir, dist server; back at the menu: 1
    let o = with_input(&e, &["init", "-q"], "2\n0.1.0\nminimal\nn\n\n\n1\n");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert_eq!(keen_version(&e), "keen 0.1.0");
    assert!(!e.home.join(".profile").exists(), "PATH modification was declined");
    assert!(!e.keen_home.join("share/completions/keen.bash").exists(), "minimal profile");
}

// ------------------------------------------------------------ update and toolchains

#[test]
fn update_default_rollback_and_uninstall_toolchains() {
    if !tools() {
        return;
    }
    let e = env("update");
    add_release(&e, "0.2.1", true);
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());

    // up to date
    let o = run(whet(&e).arg("update"));
    assert!(o.status.success());
    assert!(err(&o).contains("up to date"), "{}", err(&o));

    // a newer release appears
    add_release(&e, "0.3.0", true);
    let o = run(whet(&e).args(["update", "--check"]));
    assert!(out(&o).contains("update available: 0.2.1 -> 0.3.0"), "{}", out(&o));
    assert_eq!(keen_version(&e), "keen 0.2.1", "--check changes nothing");
    let o = run(whet(&e).arg("update"));
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("updated keen 0.2.1 -> 0.3.0"), "{}", out(&o));
    assert_eq!(keen_version(&e), "keen 0.3.0");
    let o = run(whet(&e).args(["toolchain", "list"]));
    assert_eq!(out(&o), "0.2.1\n0.3.0 (default)\n");

    // roll back, and the default cannot be removed
    assert!(run(whet(&e).args(["default", "0.2.1"])).status.success());
    assert_eq!(keen_version(&e), "keen 0.2.1");
    let o = run(whet(&e).args(["toolchain", "uninstall", "0.2.1"]));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("default toolchain"), "{}", err(&o));
    assert!(run(whet(&e).args(["toolchain", "uninstall", "0.3.0"])).status.success());
    assert_eq!(out(&run(whet(&e).args(["toolchain", "list"]))), "0.2.1 (default)\n");
    assert_eq!(run(whet(&e).args(["default", "9.9.9"])).status.code(), Some(1));
}

#[test]
fn update_never_downgrades() {
    if !tools() {
        return;
    }
    let e = env("nodown");
    add_release(&e, "0.2.1", true);
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());
    std::fs::write(e.dist.join("latest"), "0.1.0\n").unwrap();
    let o = run(whet(&e).arg("update"));
    assert!(o.status.success());
    assert!(err(&o).contains("newer than the latest"), "{}", err(&o));
    assert_eq!(keen_version(&e), "keen 0.2.1");
}

#[test]
fn show_which_run_and_check() {
    if !tools() {
        return;
    }
    let e = env("show");
    add_release(&e, "0.2.1", true);
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());
    let o = run(whet(&e).arg("show"));
    assert!(out(&o).contains("default:      0.2.1") && out(&o).contains("0.2.1 (default)"), "{}", out(&o));
    let o = run(whet(&e).arg("which"));
    assert_eq!(out(&o).trim(), e.keen_home.join("toolchains/0.2.1/bin/keen").display().to_string());
    // everything after the version belongs to the command, including `--version`
    let o = run(whet(&e).args(["run", "0.2.1", "keen", "--version"]));
    assert_eq!(out(&o).trim(), "keen 0.2.1", "{}", err(&o));
    let o = run(whet(&e).args(["run", "9.9.9", "keen"]));
    assert_eq!(o.status.code(), Some(1));
    // check: with bin on PATH the link, PATH and dist server are all fine
    let path = format!("{}:{}", e.keen_home.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    let o = run(whet(&e).env("PATH", path).args(["check", "--online"]));
    let text = out(&o);
    assert!(text.contains("keen 0.2.1") && text.contains("is on PATH") && text.contains("latest is 0.2.1"), "{}", text);
    assert!(!text.contains("FAIL ~") && !text.contains("FAIL whet") && !text.contains("FAIL default"), "{}", text);
}

#[test]
fn self_update_replaces_whet() {
    if !tools() {
        return;
    }
    let e = env("selfup");
    add_release(&e, "0.2.1", true);
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());
    let o = run(whet(&e).args(["self", "update"]));
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("up to date"), "same version as the running whet: {}", err(&o));
    let o = run(whet(&e).args(["self", "update", "--force"]));
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("updated whet"), "{}", out(&o));
}

// ------------------------------------------------------------------ uninstall

#[test]
fn self_uninstall_restores_the_shell_profiles() {
    if !tools() {
        return;
    }
    let e = env("uninstall");
    add_release(&e, "0.2.1", true);
    std::fs::create_dir_all(e.home.join(".config/fish")).unwrap();
    std::fs::write(e.home.join(".bashrc"), "export A=1\n").unwrap();
    assert!(run(whet(&e).args(["init", "-y", "-q"])).status.success());
    let fish_conf = e.home.join(".config/fish/conf.d/keen.fish");
    assert!(fish_conf.is_file(), "fish users get a conf.d snippet");
    assert!(e.home.join(".profile").is_file());

    let o = run(whet(&e).args(["self", "uninstall", "-y"]));
    assert!(o.status.success(), "{}", err(&o));
    assert!(!e.keen_home.exists());
    assert_eq!(read(&e.home.join(".bashrc")), "export A=1\n");
    assert!(!e.home.join(".profile").exists(), "a profile whet created is removed when empty");
    assert!(!fish_conf.exists());
}

#[test]
fn self_uninstall_refuses_foreign_directories() {
    let e = env("foreign");
    std::fs::create_dir_all(&e.keen_home).unwrap();
    std::fs::write(e.keen_home.join("precious.txt"), "do not delete").unwrap();
    let o = run(whet(&e).args(["self", "uninstall", "-y"]));
    assert_eq!(o.status.code(), Some(1));
    assert!(e.keen_home.join("precious.txt").exists());
}

// -------------------------------------------------------------------- bootstrap

fn install_sh() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh")
}

fn script(e: &Env) -> Command {
    let mut c = Command::new("sh");
    c.arg(install_sh())
        .env("HOME", &e.home)
        .env("KEEN_HOME", &e.keen_home)
        .env("WHET_DIST_SERVER", format!("file://{}", e.dist.display()))
        .env("WHET_TARGET", TARGET)
        .env("SHELL", "/bin/bash")
        .stdin(Stdio::null());
    c
}

#[test]
fn install_script_bootstraps_whet_and_installs_keen() {
    if !tools() {
        return;
    }
    let e = env("script");
    add_release(&e, "0.2.1", true);
    let o = run(script(&e).args(["-y", "--profile", "minimal"]));
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(err(&o).contains("downloading whet 0.2.1"), "{}", err(&o));
    assert_eq!(keen_version(&e), "keen 0.2.1");
    assert!(e.keen_home.join("bin/whet").is_file());
    assert!(!e.keen_home.join("share/completions/keen.bash").exists(), "options reach `whet init`");
}

#[test]
fn install_script_rejects_a_tampered_whet() {
    if !tools() {
        return;
    }
    let e = env("tamper");
    add_release(&e, "0.2.1", true);
    let w = e.dist.join("whet/0.2.1").join(format!("whet-{}", TARGET));
    let mut bytes = std::fs::read(&w).unwrap();
    bytes.extend_from_slice(b"malware");
    std::fs::write(&w, bytes).unwrap();
    let o = run(script(&e).arg("-y"));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("checksum mismatch"), "{}", err(&o));
    assert!(!e.keen_home.exists(), "nothing is installed or run");
}

#[test]
fn install_script_refuses_insecure_urls_and_shows_help() {
    let e = env("insecure");
    let o = run(script(&e).env("WHET_DIST_SERVER", "http://example.org/dist").arg("-y"));
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("insecure"), "{}", err(&o));
    let o = run(script(&e).arg("--help"));
    assert!(o.status.success());
    assert!(err(&o).is_empty() && out(&o).contains("--default-toolchain"), "{}", out(&o));
}

// ------------------------------------------------------------------------ misc

#[test]
fn version_help_and_completions() {
    let e = env("misc");
    assert!(out(&run(whet(&e).arg("--version"))).starts_with("whet "));
    assert!(out(&run(whet(&e).args(["help", "init"]))).contains("--no-modify-path"));
    assert!(out(&run(whet(&e).arg("help"))).contains("self uninstall"));
    assert!(out(&run(whet(&e).args(["completions", "bash"]))).contains("complete -F _whet whet"));
    assert!(out(&run(whet(&e).args(["completions", "fish"]))).contains("complete -c whet"));
    assert_eq!(run(whet(&e).arg("frobnicate")).status.code(), Some(2));
    assert_eq!(run(whet(&e).args(["completions", "nope"])).status.code(), Some(2));
    // not installed: show says so, update/default fail cleanly
    assert!(out(&run(whet(&e).arg("show"))).contains("not installed"));
    assert_eq!(run(whet(&e).arg("update")).status.code(), Some(1));
}
