//! Project workflow tests: keen.toml, new/init, incremental builds, test runner, fmt, explain.

use keen::manifest;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn keen() -> Command {
    Command::new(env!("CARGO_BIN_EXE_keen"))
}

fn have_cc() -> bool {
    let ok = ["cc", "gcc", "clang"]
        .iter()
        .any(|c| Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false));
    if !ok {
        eprintln!("no C compiler found; skipping");
    }
    ok
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("keen-proj-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
    p
}

/// Captured stdout with line endings normalized: a C program's `printf("..\n")` writes `\r\n` on
/// Windows (text mode), and the test runner's own output goes through the same stream.
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n")
}

/// `target/debug/demo` (+ `.exe` on Windows).
fn exe(project: &Path, profile: &str, name: &str) -> PathBuf {
    project.join("target").join(profile).join(format!("{}{}", name, std::env::consts::EXE_SUFFIX))
}

fn new_project(base: &Path, name: &str) -> PathBuf {
    let o = keen().current_dir(base).args(["new", name, "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    base.join(name)
}

// ------------------------------------------------------------------------ manifest

#[test]
fn manifest_parses_all_fields() {
    let m = manifest::parse(
        "# comment\n[package]\nname = \"demo-app\"\nversion = \"1.2.3\"\nentry = \"src/app.kn\"\n\n[build]\nstd = \"c23\"\ncc = \"clang\"\nopt = 2\nwarnings = false\ncflags = [\"-DA\", \"-DB=1\"] # trailing\nldflags = [\n  \"-lm\",\n  \"-lpthread\",\n]\nsources = [\"c/x.c\"]\n",
    )
    .unwrap();
    assert_eq!(m.name, "demo-app");
    assert_eq!(m.version, "1.2.3");
    assert_eq!(m.entry, "src/app.kn");
    assert_eq!(m.std.as_deref(), Some("c23"));
    assert_eq!(m.cc.as_deref(), Some("clang"));
    assert_eq!(m.opt.as_deref(), Some("2"));
    assert_eq!(m.warnings, Some(false));
    assert_eq!(m.cflags, vec!["-DA", "-DB=1"]);
    assert_eq!(m.ldflags, vec!["-lm", "-lpthread"]);
    assert_eq!(m.sources, vec!["c/x.c"]);
}

#[test]
fn manifest_defaults_and_errors() {
    let m = manifest::parse("[package]\nname = \"x\"\n").unwrap();
    assert_eq!(m.entry, "src/main.kn");
    assert_eq!(m.version, "0.1.0");
    let bad = |s: &str| manifest::parse(s).unwrap_err();
    assert!(bad("[package]\nversion = \"1\"\n").contains("missing `name`"));
    assert!(bad("[package]\nname = \"1x\"\n").contains("must start with a letter"));
    assert!(bad("[package]\nname = \"x\"\nbogus = 1\n").contains("unknown key `bogus`"));
    assert!(bad("[package]\nname = \"x\"\n[build]\nstd = \"c99\"\n").contains("c11"));
    assert!(bad("[nope]\n").contains("unknown section"));
    assert!(bad("[package]\nname = x\n").contains("keen.toml:2"));
}

// ------------------------------------------------------------------------ new / init

#[test]
fn new_creates_a_project_and_refuses_to_overwrite() {
    let d = tmp("new");
    let p = new_project(&d, "hello");
    assert!(p.join("keen.toml").is_file());
    assert!(p.join("src/main.kn").is_file());
    assert!(std::fs::read_to_string(p.join(".gitignore")).unwrap().contains("target/"));
    let o = keen().current_dir(&d).args(["new", "hello"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("already exists"), "{}", err(&o));
    let o = keen().current_dir(&d).args(["new", "9bad"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn init_uses_the_directory_name() {
    let d = tmp("init");
    let dir = d.join("my-tool");
    std::fs::create_dir_all(&dir).unwrap();
    let o = keen().current_dir(&dir).args(["init", "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let m = manifest::load(&dir).unwrap();
    assert_eq!(m.name, "my-tool");
    let o = keen().current_dir(&dir).args(["init"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
}

// ------------------------------------------------------------------- build and run

#[test]
fn run_works_from_a_subdirectory_of_the_project() {
    if !have_cc() {
        return;
    }
    let d = tmp("run");
    let p = new_project(&d, "demo");
    let o = keen().current_dir(p.join("src")).args(["run", "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "Hello from demo!\n");
    assert!(exe(&p, "debug", "demo").exists());
}

#[test]
fn builds_are_incremental() {
    if !have_cc() {
        return;
    }
    let d = tmp("incr");
    let p = new_project(&d, "demo");
    let first = keen().current_dir(&p).arg("build").output().unwrap();
    assert!(err(&first).contains("built"), "{}", err(&first));
    let second = keen().current_dir(&p).arg("build").output().unwrap();
    assert!(err(&second).contains("up to date"), "{}", err(&second));
    let src = p.join("src/main.kn");
    let text = std::fs::read_to_string(&src).unwrap().replace("Hello", "Howdy");
    std::fs::write(&src, text).unwrap();
    let third = keen().current_dir(&p).arg("build").output().unwrap();
    assert!(err(&third).contains("built"), "{}", err(&third));
    let run = keen().current_dir(&p).args(["run", "-q"]).output().unwrap();
    assert_eq!(out(&run), "Howdy from demo!\n");
}

#[test]
fn release_profile_has_its_own_directory() {
    if !have_cc() {
        return;
    }
    let d = tmp("rel");
    let p = new_project(&d, "demo");
    let o = keen().current_dir(&p).args(["build", "--release", "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(exe(&p, "release", "demo").exists());
    assert!(!exe(&p, "debug", "demo").exists());
}

#[test]
fn manifest_build_settings_are_applied() {
    if !have_cc() {
        return;
    }
    let d = tmp("settings");
    let p = d.join("cfg");
    write(
        &p,
        "keen.toml",
        "[package]\nname = \"cfgdemo\"\n\n[build]\ncflags = [\"-DFAST\"]\nldflags = [\"-lm\"]\nsources = [\"c/helper.c\"]\n",
    );
    write(&p, "c/helper.c", "int twice(int x) { return x * 2; }\n");
    write(
        &p,
        "src/main.kn",
        "use <stdio.h>;\nuse <math.h>;\nextern { fn twice(x: c_int) -> c_int; }\n#[cfg(FAST)]\nfn mode() -> i32 { 1 }\n#[cfg(not(FAST))]\nfn mode() -> i32 { 2 }\npub fn main() -> i32 {\n    printf(\"%d %d %.0f\\n\", mode(), twice(21), sqrt(16.0));\n    return 0;\n}\n",
    );
    let o = keen().current_dir(&p).args(["run", "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "1 42 4\n");
}

#[test]
fn no_project_and_no_file_is_a_usage_error() {
    let d = tmp("nothing");
    for cmd in ["build", "run", "test", "check"] {
        let o = keen().current_dir(&d).arg(cmd).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{}: {}", cmd, err(&o));
        assert!(err(&o).contains("keen.toml"), "{}", err(&o));
    }
}

#[test]
fn clean_removes_target() {
    if !have_cc() {
        return;
    }
    let d = tmp("clean");
    let p = new_project(&d, "demo");
    assert!(keen().current_dir(&p).args(["build", "-q"]).status().unwrap().success());
    assert!(p.join("target").exists());
    assert!(keen().current_dir(&p).args(["clean", "-q"]).status().unwrap().success());
    assert!(!p.join("target").exists());
}

// ------------------------------------------------------------------------- tests

fn test_project(dir: &Path, main: &str, extra: &[(&str, &str)]) -> PathBuf {
    write(dir, "keen.toml", "[package]\nname = \"t\"\n");
    write(dir, "src/main.kn", main);
    for (n, c) in extra {
        write(dir, n, c);
    }
    dir.to_path_buf()
}

// Process-per-test isolation (signals, captured assert output) needs fork(), i.e. Unix.
#[cfg(unix)]
#[test]
fn test_runner_reports_passes_and_failures() {
    if !have_cc() {
        return;
    }
    let d = tmp("tests");
    let p = test_project(
        &d,
        "use <assert.h>;\nmod util;\npub fn main() -> i32 { return 0; }\n#[test]\nfn passes() { assert(1 + 1 == 2); }\n#[test]\nfn nonzero_return_fails() -> i32 { 1 }\n#[test]\nfn zero_return_passes() -> i32 { 0 }\n#[test]\nfn assert_fails() { assert(1 == 2); }\n#[test]\nfn crashes() { let p: *mut i32 = NULL; *p = 1; }\n",
        &[("src/util.kn", "use <assert.h>;\npub fn id(x: i32) -> i32 { x }\n#[test]\nfn id_works() { assert(id(3) == 3); }\n")],
    );
    let o = keen().current_dir(&p).arg("test").output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let text = out(&o);
    assert!(text.contains("running 6 tests"), "{}", text);
    assert!(text.contains("test passes ... ok"), "{}", text);
    assert!(text.contains("test zero_return_passes ... ok"), "{}", text);
    assert!(text.contains("test util::id_works ... ok"), "{}", text);
    assert!(text.contains("test nonzero_return_fails ... FAILED"), "{}", text);
    assert!(text.contains("test assert_fails ... FAILED"), "{}", text);
    assert!(text.contains("Assertion"), "{}", text);
    assert!(text.contains("test crashes ... FAILED"), "{}", text);
    assert!(text.contains("signal 11"), "{}", text);
    assert!(text.contains("test result: FAILED. 3 passed; 3 failed"), "{}", text);
}

#[test]
fn test_runner_filters_and_lists() {
    if !have_cc() {
        return;
    }
    let d = tmp("filter");
    let p = test_project(
        &d,
        "use <assert.h>;\npub fn main() -> i32 { return 0; }\n#[test]\nfn alpha() { assert(1); }\n#[test]\nfn beta() { assert(1); }\n",
        &[],
    );
    let o = keen().current_dir(&p).args(["test", "alp"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("running 1 test\n") && out(&o).contains("alpha"), "{}", out(&o));
    assert!(!out(&o).contains("beta"));
    let o = keen().current_dir(&p).args(["test", "--", "--list"]).output().unwrap();
    assert_eq!(out(&o), "alpha\nbeta\n");
}

#[test]
fn test_functions_are_not_part_of_normal_builds() {
    if !have_cc() {
        return;
    }
    let d = tmp("notest");
    let p = test_project(
        &d,
        "use <stdio.h>;\npub fn main() -> i32 { printf(\"main\\n\"); return 0; }\n#[test]\nfn helper() { this_is_not_defined(); }\n",
        &[],
    );
    let o = keen().current_dir(&p).args(["run", "-q", "-w"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "main\n");
}

#[test]
fn invalid_test_function_is_rejected() {
    let d = tmp("badtest");
    let f = write(&d, "t.kn", "#[test]\nfn needs_arg(x: i32) {}\n");
    let o = keen().arg("emit").arg(&f).args(["-o", "-"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("E0502"), "{}", err(&o));
}

#[test]
fn loose_file_test_mode() {
    if !have_cc() {
        return;
    }
    let d = tmp("loose-test");
    let f = write(&d, "unit.kn", "use <assert.h>;\n#[test]\nfn it_works() { assert(2 * 2 == 4); }\n");
    let o = keen().arg("test").arg(&f).output().unwrap();
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("test it_works ... ok"), "{}", out(&o));
}

// -------------------------------------------------------------------------- fmt

#[test]
fn fmt_formats_a_project_in_place_and_check_mode_agrees() {
    let d = tmp("fmt");
    let p = test_project(
        &d,
        "pub fn main()->i32{return 0;}\n",
        &[("src/sub/util.kn", "pub fn id(x:i32)->i32{x}\n"), ("target/ignored.kn", "this is not valid keen")],
    );
    let o = keen().current_dir(&p).args(["fmt", "--check"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("Diff in"), "{}", out(&o));
    let o = keen().current_dir(&p).arg("fmt").output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(p.join("src/main.kn")).unwrap(),
        "pub fn main() -> i32 {\n    return 0;\n}\n"
    );
    assert_eq!(std::fs::read_to_string(p.join("src/sub/util.kn")).unwrap(), "pub fn id(x: i32) -> i32 { x }\n");
    let o = keen().current_dir(&p).args(["fmt", "--check"]).output().unwrap();
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
}

#[test]
fn fmt_stdin_and_errors() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = keen().args(["fmt", "--stdin"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"fn  f( a:i32 )->i32{a}").unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    assert_eq!(out(&o), "fn f(a: i32) -> i32 { a }\n");

    let d = tmp("fmt-err");
    let f = write(&d, "bad.kn", "fn f( {");
    let o = keen().arg("fmt").arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("error[E0101]"), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "fn f( {", "files with errors are left untouched");
}

// ---------------------------------------------------------------- explain / help

#[test]
fn explain_lists_and_describes_codes() {
    let o = keen().arg("explain").output().unwrap();
    assert!(out(&o).contains("E0201") && out(&o).contains("E0502"), "{}", out(&o));
    let o = keen().args(["explain", "e0601"]).output().unwrap();
    assert!(o.status.success());
    assert!(out(&o).contains("by-value cycle"), "{}", out(&o));
    let o = keen().args(["explain", "E9999"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn every_diagnostic_code_used_by_keen_is_explained() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut codes = std::collections::BTreeSet::new();
    for f in ["emit.rs", "parser.rs", "lexer.rs", "fmt.rs", "driver.rs"] {
        let text = std::fs::read_to_string(src_dir.join(f)).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("\"E0") {
            let code: String = rest[i + 1..].chars().take(5).collect();
            if code.len() == 5 && code[1..].chars().all(|c| c.is_ascii_digit()) {
                codes.insert(code);
            }
            rest = &rest[i + 3..];
        }
    }
    assert!(codes.len() >= 12, "{:?}", codes);
    for c in codes {
        assert!(keen::explain::lookup(&c).is_some(), "{} has no `keen explain` entry", c);
    }
}

#[test]
fn completions_and_help() {
    for sh in ["bash", "zsh", "fish"] {
        let o = keen().args(["completions", sh]).output().unwrap();
        assert!(o.status.success(), "{}", sh);
        assert!(out(&o).contains("build") && out(&o).contains("fmt"), "{}: {}", sh, out(&o));
    }
    assert_eq!(keen().args(["completions", "powershell"]).output().unwrap().status.code(), Some(2));
    let o = keen().args(["build", "--help"]).output().unwrap();
    assert!(out(&o).contains("keen build"), "{}", out(&o));
    let o = keen().args(["help", "fmt"]).output().unwrap();
    assert!(out(&o).contains("--check"), "{}", out(&o));
    let o = keen().arg("--version").output().unwrap();
    assert!(out(&o).starts_with("keen "));
}

#[test]
fn diagnostics_show_a_source_snippet() {
    let d = tmp("snippet");
    let f = write(&d, "bad.kn", "pub fn main() -> i32 {\n    let a: i32 = 1;\n    if a < 1 < 2 { return 1; }\n    return 0;\n}\n");
    let o = keen().arg("check").arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let log = err(&o);
    assert!(log.contains("error[E0201]"), "{}", log);
    assert!(log.contains("3 |     if a < 1 < 2 { return 1; }"), "{}", log);
    assert!(log.contains('^') && log.contains("keen explain E0201"), "{}", log);
}

// ------------------------------------------------------------------ line endings

#[test]
fn fmt_preserves_crlf_and_check_agrees() {
    let d = tmp("crlf-fmt");
    let f = write(&d, "a.kn", "pub fn main()->i32{\r\nreturn 0;}\r\n");
    let o = keen().arg("fmt").arg(&f).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let bytes = std::fs::read(&f).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text, "pub fn main() -> i32 {\r\n    return 0;\r\n}\r\n");
    assert!(keen().args(["fmt", "--check"]).arg(&f).status().unwrap().success());

    // LF files stay LF
    let g = write(&d, "b.kn", "pub fn main()->i32{return 0;}\n");
    assert!(keen().arg("fmt").arg(&g).status().unwrap().success());
    assert!(!std::fs::read_to_string(&g).unwrap().contains('\r'));
}

#[test]
fn crlf_manifest_and_sources_build_the_same_program() {
    if !have_cc() {
        return;
    }
    let d = tmp("crlf-proj");
    let crlf = |s: &str| s.replace('\n', "\r\n");
    write(&d, "keen.toml", &crlf("[package]\nname = \"crlf\"\n[build]\ncflags = [\n  \"-DFAST\",\n]\n"));
    write(
        &d,
        "src/main.kn",
        &crlf("use <stdio.h>;\n// a comment\n#[cfg(FAST)]\nfn mode() -> i32 { 1 }\n#[cfg(not(FAST))]\nfn mode() -> i32 { 2 }\npub fn main() -> i32 {\n    printf(\"%d\\n\", mode());\n    return 0;\n}\n"),
    );
    let o = keen().current_dir(&d).args(["run", "-q"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "1\n");
}
