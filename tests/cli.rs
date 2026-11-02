//! End-to-end tests of the `keen` binary: build/run/check driving a real C compiler.
//! Tests are skipped (with a note) when no C compiler is installed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn keen() -> Command {
    Command::new(env!("CARGO_BIN_EXE_keen"))
}

fn have(cc: &str) -> bool {
    Command::new(cc).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn have_cc() -> bool {
    let ok = have("cc") || have("gcc") || have("clang");
    if !ok {
        eprintln!("no C compiler found; skipping");
    }
    ok
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("keen-cli-{}-{}", name, std::process::id()));
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

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(name)
}

/// Captured stdout with line endings normalized: a C program's `printf("..\n")` writes `\r\n` on
/// Windows (text mode), and the test runner's own output goes through the same stream.
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n")
}

#[test]
fn run_prints_program_output() {
    if !have_cc() {
        return;
    }
    let o = keen().arg("run").arg(example("hello.kn")).arg("-q").output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "hello from keen\n");
}

#[test]
fn run_forwards_the_exit_code() {
    if !have_cc() {
        return;
    }
    let d = tmp("exit");
    let f = write(&d, "seven.kn", "pub fn main() -> i32 { return 7; }\n");
    let o = keen().arg("run").arg(&f).arg("-q").output().unwrap();
    assert_eq!(o.status.code(), Some(7));
}

#[test]
fn run_passes_arguments_after_double_dash() {
    if !have_cc() {
        return;
    }
    let d = tmp("args");
    let f = write(
        &d,
        "args.kn",
        "use <stdio.h>;\npub fn main(argc: i32, argv: *mut *mut c_char) -> i32 {\n    printf(\"%d %s %s\\n\", argc - 1, argv[1], argv[2]);\n    return 0;\n}\n",
    );
    let o = keen().arg("run").arg(&f).arg("-q").arg("--").arg("one").arg("two").output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "2 one two\n");
}

#[test]
fn build_writes_an_executable_and_keeps_generated_c() {
    if !have_cc() {
        return;
    }
    let d = tmp("build");
    write(&d, "hello.kn", &std::fs::read_to_string(example("hello.kn")).unwrap());
    let o = keen().current_dir(&d).args(["build", "hello.kn"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("built"), "{}", err(&o));
    let exe = d.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.exists());
    assert!(d.join(".keen/hello/hello.c").exists());
    let run = Command::new(&exe).output().unwrap();
    assert_eq!(out(&run), "hello from keen\n");

    let o = keen().current_dir(&d).args(["clean", "-q"]).output().unwrap();
    assert!(o.status.success());
    assert!(!d.join(".keen").exists());
}

#[test]
fn build_honours_output_path_and_release_flags() {
    if !have_cc() {
        return;
    }
    let d = tmp("release");
    let f = write(&d, "hello.kn", &std::fs::read_to_string(example("hello.kn")).unwrap());
    let exe = d.join("bin").join("app");
    let o = keen()
        .args(["build", "--release", "-v", "-q"])
        .arg(&f)
        .arg("-o")
        .arg(&exe)
        .arg("--out-dir")
        .arg(d.join("gen"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let log = err(&o);
    assert!(log.contains("-O2"), "{}", log);
    assert!(!log.contains(" -g "), "{}", log);
    assert!(d.join("gen/hello.c").exists());
    assert_eq!(out(&Command::new(&exe).output().unwrap()), "hello from keen\n");
}

#[test]
fn check_reports_errors_at_kn_lines() {
    if !have_cc() {
        return;
    }
    let d = tmp("check");
    let f = write(&d, "bad.kn", "pub fn main() -> i32 {\n    let x: i32 = 1;\n    x = 2;\n    return x;\n}\n");
    let o = keen().arg("check").arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let log = err(&o);
    assert!(log.contains("bad.kn:3"), "expected a bad.kn:3 diagnostic, got:\n{}", log);
}

#[test]
fn check_succeeds_on_valid_code() {
    if !have_cc() {
        return;
    }
    let o = keen().arg("check").arg(example("features.kn")).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn transpile_errors_use_diagnostic_codes() {
    let d = tmp("diag");
    let f = write(&d, "oops.kn", "fn f(a: i32) -> bool { a < 1 < 2 }\n");
    let o = keen().arg("emit").arg(&f).arg("-o").arg("-").output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("E0201"), "{}", err(&o));
}

#[test]
fn missing_compiler_is_a_clear_error() {
    let o = keen().args(["build", "--cc", "definitely-not-a-compiler-xyz"]).arg(example("hello.kn")).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("could not be run"), "{}", err(&o));
}

#[test]
fn keen_cc_environment_variable_is_used() {
    let o = keen()
        .env("KEEN_CC", "definitely-not-a-compiler-xyz")
        .args(["check"])
        .arg(example("hello.kn"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("KEEN_CC"), "{}", err(&o));
}

#[test]
fn defines_work_with_cfg() {
    if !have_cc() {
        return;
    }
    let d = tmp("define");
    let f = write(
        &d,
        "mode.kn",
        "use <stdio.h>;\n#[cfg(FAST)]\nfn mode() -> i32 { 1 }\n#[cfg(not(FAST))]\nfn mode() -> i32 { 2 }\npub fn main() -> i32 {\n    printf(\"%d\\n\", mode());\n    return 0;\n}\n",
    );
    let a = keen().args(["run", "-q"]).arg(&f).output().unwrap();
    assert_eq!(out(&a), "2\n");
    let b = keen().args(["run", "-q", "-D", "FAST"]).arg(&f).output().unwrap();
    assert_eq!(out(&b), "1\n");
    let c = keen().args(["run", "-q", "-DFAST"]).arg(&f).output().unwrap();
    assert_eq!(out(&c), "1\n");
}

#[test]
fn libraries_can_be_linked() {
    if !have_cc() {
        return;
    }
    let d = tmp("libm");
    let f = write(
        &d,
        "root.kn",
        "use <stdio.h>;\nextern { fn sqrt(x: f64) -> f64; }\npub fn main() -> i32 {\n    printf(\"%.0f\\n\", sqrt(16.0));\n    return 0;\n}\n",
    );
    let o = keen().args(["run", "-q", "-l", "m"]).arg(&f).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "4\n");
}

#[test]
fn extra_c_files_are_linked() {
    if !have_cc() {
        return;
    }
    let d = tmp("extra");
    let k = write(
        &d,
        "main.kn",
        "use <stdio.h>;\nextern { fn twice(x: c_int) -> c_int; }\npub fn main() -> i32 {\n    printf(\"%d\\n\", twice(21));\n    return 0;\n}\n",
    );
    let c = write(&d, "helper.c", "int twice(int x) { return x * 2; }\n");
    let o = keen().args(["run", "-q"]).arg(&k).arg(&c).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "42\n");
}

#[test]
fn multi_file_project_with_nested_modules() {
    if !have_cc() {
        return;
    }
    let o = keen().args(["run", "-q"]).arg(example("project/main.kn")).args(["--", "a", "b"]).output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(text.contains("area=12 perimeter=14"), "{}", text);
    assert!(text.contains("clamped=100 args=2"), "{}", text);
    assert!(text.contains("KEEN PROJECT"), "{}", text);
}

#[test]
fn missing_module_file_is_reported() {
    let d = tmp("nomod");
    let f = write(&d, "main.kn", "mod ghost;\npub fn main() -> i32 { return 0; }\n");
    let o = keen().arg("check").arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let log = err(&o);
    assert!(log.contains("ghost") && log.contains("not found"), "{}", log);
}

#[test]
fn emit_command_and_legacy_form() {
    let d = tmp("emit");
    let f = write(&d, "a.kn", "pub fn main() -> i32 { return 3; }\n");
    let o = keen().arg("emit").arg(&f).args(["-o", "-", "--no-line-directives"]).output().unwrap();
    assert!(o.status.success());
    assert!(out(&o).contains("int32_t main(void)"), "{}", out(&o));
    let o = keen().arg(&f).output().unwrap(); // legacy: writes a.c next to the input
    assert!(o.status.success(), "{}", err(&o));
    assert!(d.join("a.c").exists());
}

#[test]
fn doctor_reports_the_compiler() {
    if !have_cc() {
        return;
    }
    let o = keen().arg("doctor").output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("C compiler:"), "{}", out(&o));
}

#[test]
fn usage_errors_exit_with_2() {
    let o = keen().args(["build", "--bogus"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let o = keen().args(["run"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let o = keen().args(["emit", "x.kn", "--", "arg"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn clang_works_too() {
    if !have("clang") {
        eprintln!("clang not installed; skipping");
        return;
    }
    for f in ["hello.kn", "tree.kn", "features.kn", "edge.kn"] {
        let o = keen().args(["run", "-q", "--cc", "clang"]).arg(example(f)).output().unwrap();
        assert!(o.status.success(), "{} failed under clang:\n{}", f, err(&o));
    }
}
