//! Transpile every file in `examples/` and make sure gcc accepts the result.
//! Skipped (with a note) when no C compiler is available.

use std::path::Path;
use std::process::Command;

fn have_gcc() -> bool {
    Command::new("gcc").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn examples_compile_with_gcc() {
    if !have_gcc() {
        eprintln!("gcc not found; skipping");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let tmp = std::env::temp_dir().join("keen-example-tests");
    std::fs::create_dir_all(&tmp).unwrap();
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("kn") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let opts = keen::Options { file: path.display().to_string(), stem: name.clone(), ..Default::default() };
        let out = keen::transpile(&src, &opts).unwrap_or_else(|e| panic!("{}: {}", name, e));
        let cfile = tmp.join(format!("{}.c", name));
        std::fs::write(&cfile, out.c).unwrap();
        let status = Command::new("gcc")
            .args(["-std=gnu11", "-Wall", "-Wextra", "-Wno-unused-parameter", "-Wno-unused-local-typedefs", "-Werror", "-fsyntax-only"])
            .arg(&cfile)
            .status()
            .unwrap();
        assert!(status.success(), "gcc rejected the output for {}", name);
        n += 1;
    }
    assert!(n >= 3, "expected at least three examples");
}
