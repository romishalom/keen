mod cli;
mod completions;

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::atomic::Ordering;

use cli::{Cli, Cmd};
use keen::driver::{self, CcOptions};
use keen::error::COLOR;
use keen::{explain, fmt as kfmt, manifest};

// ------------------------------------------------------------------ small helpers

fn paint(s: &str, code: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

/// Print an error. Pre-rendered diagnostics (from the driver) are printed as they are.
fn err(msg: &str) -> i32 {
    if msg.contains("error[") || msg.contains(": error") {
        eprint!("{}", msg);
        if !msg.ends_with('\n') {
            eprintln!();
        }
    } else {
        eprintln!("{} {}", paint("keen: error:", "1;31"), msg);
    }
    1
}

fn usage_err(msg: &str) -> i32 {
    eprintln!("{} {}", paint("keen:", "1;31"), msg);
    eprintln!("try `keen help`");
    2
}

fn note(cli: &Cli, msg: &str) {
    if !cli.quiet {
        eprintln!("{} {}", paint("keen:", "1;32"), msg);
    }
}

fn exe_name(stem: &str) -> String {
    format!("{}{}", stem, std::env::consts::EXE_SUFFIX)
}

fn stem_of(p: &str) -> String {
    Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".to_string())
}

fn ext_of(p: &str) -> String {
    Path::new(p).extension().and_then(|e| e.to_str()).unwrap_or("").to_string()
}

/// Removes a directory when dropped (temporary build directories).
struct Cleanup {
    dir: PathBuf,
    remove: bool,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.remove {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

// --------------------------------------------------------------------- projects

struct Proj {
    root: PathBuf,
    m: manifest::Manifest,
}

/// For commands that work on a project: find keen.toml and move into its directory.
fn prepare(cli: &mut Cli) -> Result<Option<Proj>, String> {
    if !matches!(cli.cmd, Cmd::Build | Cmd::Run | Cmd::Test | Cmd::Check | Cmd::Fmt | Cmd::Clean) {
        return Ok(None);
    }
    if cli.inputs.iter().any(|i| ext_of(i) == "kn") {
        return Ok(None);
    }
    if cli.cmd == Cmd::Fmt && !cli.inputs.is_empty() {
        return Ok(None);
    }
    let cwd = std::env::current_dir().map_err(|e| format!("cannot determine the current directory: {}", e))?;
    let Some(root) = manifest::find_root(&cwd) else { return Ok(None) };
    let m = manifest::load(&root)?;
    let abs = |s: &str| cwd.join(s).display().to_string();
    if cli.cmd != Cmd::Test {
        for i in cli.inputs.iter_mut() {
            *i = abs(i);
        }
    } else {
        for i in cli.inputs.iter_mut() {
            if matches!(ext_of(i).as_str(), "c" | "o" | "a") {
                *i = abs(i);
            }
        }
    }
    if let Some(o) = cli.output.as_mut() {
        *o = abs(o);
    }
    if let Some(o) = cli.out_dir.as_mut() {
        *o = abs(o);
    }
    std::env::set_current_dir(&root).map_err(|e| format!("cannot enter `{}`: {}", root.display(), e))?;
    Ok(Some(Proj { root, m }))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Build,
    Run,
    Test,
    Check,
}

struct Plan {
    entry: PathBuf,
    out_dir: PathBuf,
    exe: PathBuf,
    topts: keen::Options,
    cco: CcOptions,
    profile: &'static str,
    filters: Vec<String>,
    _guard: Option<Cleanup>,
}

fn make_plan(cli: &Cli, proj: &Option<Proj>, kind: Kind) -> Result<Plan, String> {
    let mut kn: Vec<String> = Vec::new();
    let mut extra: Vec<PathBuf> = Vec::new();
    let mut filters: Vec<String> = Vec::new();
    for i in &cli.inputs {
        match ext_of(i).as_str() {
            "kn" => kn.push(i.clone()),
            "c" | "o" | "a" | "h" => extra.push(PathBuf::from(i)),
            _ if kind == Kind::Test => filters.push(i.clone()),
            _ => extra.push(PathBuf::from(i)),
        }
    }
    if kn.len() > 1 {
        return Err("only one .kn entry file can be given; other keen files are found through `mod name;`".into());
    }
    let (entry, name, project_mode) = match (kn.first(), proj) {
        (Some(f), _) => (PathBuf::from(f), stem_of(f), false),
        (None, Some(p)) => (PathBuf::from(&p.m.entry), p.m.name.clone(), true),
        (None, None) => {
            return Err("no input file given and no keen.toml found (expected a .kn file, or run `keen new <name>`)".into())
        }
    };
    let stem = stem_of(&entry.display().to_string());
    let profile = if cli.release { "release" } else { "debug" };
    let pm = proj.as_ref().filter(|_| project_mode).map(|p| &p.m);

    // directories
    let temp_dir = || std::env::temp_dir().join(format!("keen-{}-{}-{}", stem, profile, std::process::id()));
    let mut guard = None;
    let persistent = |kind: Kind| -> Option<PathBuf> {
        match (project_mode, kind) {
            (true, Kind::Build | Kind::Run) => Some(PathBuf::from(format!("target/keen/{}", profile))),
            (true, Kind::Test) => Some(PathBuf::from(format!("target/keen/{}-test", profile))),
            (false, Kind::Build) => Some(PathBuf::from(format!(".keen/{}", stem))),
            _ => None,
        }
    };
    let out_dir = match (&cli.out_dir, persistent(kind)) {
        (Some(d), _) => PathBuf::from(d),
        (None, Some(d)) => d,
        (None, None) => {
            let d = temp_dir();
            guard = Some(Cleanup { dir: d.clone(), remove: !cli.keep });
            d
        }
    };
    let incremental = persistent(kind).is_some() || cli.out_dir.is_some();

    let proj_exe = |suffix: &str| PathBuf::from(format!("target/{}/{}", profile, exe_name(&format!("{}{}", name, suffix))));
    let exe = match kind {
        Kind::Build => match &cli.output {
            Some(o) => PathBuf::from(o),
            None if project_mode => proj_exe(""),
            None => PathBuf::from(exe_name(&stem)),
        },
        Kind::Run => {
            if project_mode {
                proj_exe("")
            } else {
                out_dir.join(exe_name(&stem))
            }
        }
        Kind::Test => {
            if project_mode {
                proj_exe("-test")
            } else {
                out_dir.join(exe_name(&format!("{}-test", stem)))
            }
        }
        Kind::Check => PathBuf::new(),
    };
    if kind != Kind::Check {
        if let Some(parent) = exe.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| format!("cannot create `{}`: {}", parent.display(), e))?;
            }
        }
    }

    let mut topts = cli.topts.clone();
    if cli.std_explicit.is_none() {
        if let Some(m) = pm {
            topts.std23 = m.std.as_deref() == Some("c23");
        }
    }
    topts.test = kind == Kind::Test;
    topts.split = false;

    let mut sources: Vec<PathBuf> = pm.map(|m| m.sources.iter().map(PathBuf::from).collect()).unwrap_or_default();
    sources.extend(extra);
    let mut cflags = pm.map(|m| m.cflags.clone()).unwrap_or_default();
    cflags.extend(cli.cflags.iter().cloned());
    let mut ldflags = pm.map(|m| m.ldflags.clone()).unwrap_or_default();
    ldflags.extend(cli.ldflags.iter().cloned());
    let cco = CcOptions {
        cc: cli.cc.clone().or_else(|| pm.and_then(|m| m.cc.clone())),
        release: cli.release,
        opt: cli.opt.clone().or_else(|| pm.and_then(|m| m.opt.clone())),
        debug: cli.debug,
        warnings: cli.warnings && pm.and_then(|m| m.warnings).unwrap_or(true),
        cflags,
        ldflags,
        extra_inputs: sources,
        verbose: cli.verbose,
        incremental,
    };
    Ok(Plan { entry, out_dir, exe, topts, cco, profile, filters, _guard: guard })
}

// ---------------------------------------------------------------------- commands

fn cmd_build(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let plan = match make_plan(cli, proj, Kind::Build) {
        Ok(p) => p,
        Err(e) => return usage_err(&e),
    };
    match driver::build(&plan.entry, &plan.topts, &plan.cco, &plan.out_dir, &plan.exe) {
        Ok(b) if b.up_to_date => {
            note(cli, &format!("{} is up to date", plan.exe.display()));
            0
        }
        Ok(b) => {
            note(cli, &format!("built {} ({}, {})", plan.exe.display(), b.cc.kind_name(), plan.profile));
            0
        }
        Err(e) => err(&e),
    }
}

fn run_exe(cli: &Cli, plan: &Plan, args: &[String]) -> i32 {
    match driver::run_program(&plan.exe, args, cli.verbose) {
        Ok(code) => code,
        Err(e) => err(&e),
    }
}

fn cmd_run(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let plan = match make_plan(cli, proj, Kind::Run) {
        Ok(p) => p,
        Err(e) => return usage_err(&e),
    };
    if let Err(e) = std::fs::create_dir_all(&plan.out_dir) {
        return err(&format!("cannot create `{}`: {}", plan.out_dir.display(), e));
    }
    if let Err(e) = driver::build(&plan.entry, &plan.topts, &plan.cco, &plan.out_dir, &plan.exe) {
        return err(&e);
    }
    if cli.keep && !cli.quiet {
        note(cli, &format!("kept build directory {}", plan.out_dir.display()));
    }
    run_exe(cli, &plan, &cli.prog_args)
}

fn cmd_test(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let plan = match make_plan(cli, proj, Kind::Test) {
        Ok(p) => p,
        Err(e) => return usage_err(&e),
    };
    if let Err(e) = std::fs::create_dir_all(&plan.out_dir) {
        return err(&format!("cannot create `{}`: {}", plan.out_dir.display(), e));
    }
    if let Err(e) = driver::build(&plan.entry, &plan.topts, &plan.cco, &plan.out_dir, &plan.exe) {
        return err(&e);
    }
    let mut args = plan.filters.clone();
    args.extend(cli.prog_args.iter().cloned());
    run_exe(cli, &plan, &args)
}

fn cmd_check(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let plan = match make_plan(cli, proj, Kind::Check) {
        Ok(p) => p,
        Err(e) => return usage_err(&e),
    };
    match driver::check(&plan.entry, &plan.topts, &plan.cco, &plan.out_dir) {
        Ok(cc) => {
            note(cli, &format!("{} ok (checked with {})", plan.entry.display(), cc.kind_name()));
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_emit(cli: &Cli) -> i32 {
    let mut opts = cli.topts.clone();
    let Some(input) = cli.inputs.first() else {
        return usage_err("no input file given (expected a .kn file)");
    };
    if cli.inputs.len() > 1 {
        return usage_err("`emit` takes exactly one input file");
    }
    let src = match std::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => return err(&format!("cannot read `{}`: {}", input, e)),
    };
    let in_path = Path::new(input);
    let out_path: PathBuf = match &cli.output {
        Some(o) => PathBuf::from(o),
        None => in_path.with_extension("c"),
    };
    let to_stdout = out_path.as_os_str() == "-";
    opts.file = input.clone();
    opts.stem = if to_stdout { stem_of(input) } else { stem_of(&out_path.to_string_lossy()) };
    let out = match keen::transpile(&src, &opts) {
        Ok(o) => o,
        Err(d) => {
            eprint!("{}", d.render_with_source(input, &src));
            return 1;
        }
    };
    if to_stdout {
        if let Some(h) = &out.h {
            println!("/* ===== {}.h ===== */\n{}", opts.stem, h);
            println!("/* ===== {}.c ===== */", opts.stem);
        }
        print!("{}", out.c);
        return 0;
    }
    if let Err(e) = std::fs::write(&out_path, &out.c) {
        return err(&format!("cannot write `{}`: {}", out_path.display(), e));
    }
    if let Some(h) = &out.h {
        let hp = out_path.with_extension("h");
        if let Err(e) = std::fs::write(&hp, h) {
            return err(&format!("cannot write `{}`: {}", hp.display(), e));
        }
    }
    0
}

fn collect_kn(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(rd) = std::fs::read_dir(path) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            collect_kn(&p, out);
        } else if p.extension().map_or(false, |e| e == "kn") {
            out.push(p);
        }
    }
}

fn cmd_fmt(cli: &Cli, proj: &Option<Proj>) -> i32 {
    if cli.fmt_stdin || cli.inputs.iter().any(|i| i == "-") {
        let mut src = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut src) {
            return err(&format!("cannot read stdin: {}", e));
        }
        return match kfmt::format_keep_eol(&src) {
            Ok(out) => {
                print!("{}", out);
                0
            }
            Err(d) => {
                eprint!("{}", d.render_with_source("<stdin>", &src));
                1
            }
        };
    }
    let mut files: Vec<PathBuf> = Vec::new();
    if cli.inputs.is_empty() {
        let base = proj.as_ref().map(|p| p.root.clone()).unwrap_or_else(|| PathBuf::from("."));
        collect_kn(&base, &mut files);
    } else {
        for i in &cli.inputs {
            let p = Path::new(i);
            if !p.exists() {
                return err(&format!("`{}` does not exist", i));
            }
            collect_kn(p, &mut files);
        }
    }
    if cli.fmt_stdout && files.len() > 1 {
        return usage_err("`--stdout` works with a single file");
    }
    let (mut changed, mut failed) = (0usize, 0usize);
    for f in &files {
        let shown = f.display().to_string();
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                err(&format!("cannot read `{}`: {}", shown, e));
                failed += 1;
                continue;
            }
        };
        let out = match kfmt::format_keep_eol(&src) {
            Ok(o) => o,
            Err(d) => {
                eprint!("{}", d.render_with_source(&shown, &src));
                failed += 1;
                continue;
            }
        };
        if cli.fmt_stdout {
            print!("{}", out);
            continue;
        }
        if out != src {
            changed += 1;
            if cli.fmt_check {
                println!("{} {}", paint("Diff in", "1"), shown);
                let (a, b) = (src.replace("\r\n", "\n"), out.replace("\r\n", "\n"));
                if a == b {
                    println!("(only the line endings differ)");
                } else {
                    print!("{}", kfmt::diff(&a, &b));
                }
            } else if let Err(e) = std::fs::write(f, &out) {
                err(&format!("cannot write `{}`: {}", shown, e));
                failed += 1;
            } else if !cli.quiet {
                println!("formatted {}", shown);
            }
        }
    }
    if cli.fmt_check {
        if changed > 0 {
            eprintln!("keen: {} file{} would be reformatted", changed, if changed == 1 { "" } else { "s" });
        }
        return if changed > 0 || failed > 0 { 1 } else { 0 };
    }
    if failed > 0 {
        1
    } else {
        0
    }
}

fn valid_name(n: &str) -> bool {
    n.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn scaffold(dir: &Path, name: &str, cli: &Cli) -> Result<(), String> {
    let w = |p: PathBuf, text: String| -> Result<(), String> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create `{}`: {}", parent.display(), e))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("cannot write `{}`: {}", p.display(), e))
    };
    w(dir.join("keen.toml"), manifest::template(name))?;
    if !dir.join("src/main.kn").exists() {
        w(dir.join("src/main.kn"), manifest::main_template(name))?;
    }
    let gi = dir.join(".gitignore");
    let existing = std::fs::read_to_string(&gi).unwrap_or_default();
    if !existing.lines().any(|l| l.trim() == "target/" || l.trim() == "target") {
        let mut t = existing;
        if !t.is_empty() && !t.ends_with('\n') {
            t.push('\n');
        }
        t.push_str("target/\n.keen/\n");
        w(gi, t)?;
    }
    if !dir.join("README.md").exists() {
        w(dir.join("README.md"), format!("# {}\n\nBuilt with [keen](https://github.com/).\n\n    keen run      # build and run\n    keen test     # run #[test] functions\n    keen fmt      # format the sources\n", name))?;
    }
    note(cli, &format!("created binary project `{}`", name));
    Ok(())
}

fn cmd_new(cli: &Cli) -> i32 {
    let Some(name) = cli.inputs.first() else {
        return usage_err("`keen new` needs a project name");
    };
    let base = stem_of(name);
    if !valid_name(&base) {
        return usage_err(&format!("`{}` is not a valid project name (letters, digits, `_`, `-`; must not start with a digit)", base));
    }
    let dir = PathBuf::from(name);
    if dir.exists() {
        return err(&format!("destination `{}` already exists (use `keen init` inside it)", dir.display()));
    }
    match scaffold(&dir, &base, cli) {
        Ok(()) => {
            if !cli.quiet {
                eprintln!("     next: cd {} && keen run", name);
            }
            0
        }
        Err(e) => err(&e),
    }
}

fn cmd_init(cli: &Cli) -> i32 {
    let dir = PathBuf::from(".");
    if dir.join("keen.toml").exists() {
        return err("`keen.toml` already exists here");
    }
    let name = match cli.inputs.first() {
        Some(n) => n.clone(),
        None => std::env::current_dir()
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "app".to_string()),
    };
    if !valid_name(&name) {
        return usage_err(&format!("`{}` is not a valid project name; pass one: `keen init <name>`", name));
    }
    match scaffold(&dir, &name, cli) {
        Ok(()) => 0,
        Err(e) => err(&e),
    }
}

fn cmd_clean(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match (&cli.out_dir, proj) {
        (Some(d), _) => dirs.push(PathBuf::from(d)),
        _ => {
            dirs.push(PathBuf::from(".keen"));
            if proj.is_some() {
                dirs.push(PathBuf::from("target"));
            }
        }
    }
    let mut removed = false;
    for d in dirs {
        if d.exists() {
            if let Err(e) = std::fs::remove_dir_all(&d) {
                return err(&format!("cannot remove `{}`: {}", d.display(), e));
            }
            note(cli, &format!("removed {}", d.display()));
            removed = true;
        }
    }
    if !removed {
        note(cli, "nothing to clean");
    }
    0
}

fn cmd_doctor(cli: &Cli) -> i32 {
    println!("keen {}", env!("CARGO_PKG_VERSION"));
    if let Ok(cwd) = std::env::current_dir() {
        match manifest::find_root(&cwd) {
            Some(root) => match manifest::load(&root) {
                Ok(m) => println!("project:    {} {} ({})", m.name, m.version, root.display()),
                Err(e) => println!("project:    {} (invalid: {})", root.display(), e),
            },
            None => println!("project:    none (no keen.toml found)"),
        }
    }
    for var in ["KEEN_CC", "CC"] {
        if let Ok(v) = std::env::var(var) {
            println!("${}: {}", var, v);
        }
    }
    let cc = match driver::find_cc(cli.cc.as_deref()) {
        Ok(cc) => cc,
        Err(e) => {
            println!("C compiler: not found");
            return err(&e);
        }
    };
    println!("C compiler: {} [{}]", cc.display(), cc.kind_name());
    println!("  version:  {}", cc.version);
    let c11 = driver::probe_std(
        &cc,
        "gnu11",
        "int main(void) { __auto_type x = 1; switch (x) { case 1 ... 2: break; } return 0; }\n",
    );
    let c23 = driver::probe_std(&cc, "gnu2x", "int main(void) { auto x = 1; return x - 1; }\n");
    println!("  --std=c11 (gnu11): {}", if c11 { "ok" } else { "NOT supported (needs __auto_type and case ranges)" });
    println!("  --std=c23 (gnu2x): {}", if c23 { "ok" } else { "not supported by this compiler version" });
    let others: Vec<&str> = ["gcc", "clang", "cc"]
        .into_iter()
        .filter(|c| *c != cc.program && driver::find_cc(Some(c)).is_ok())
        .collect();
    if !others.is_empty() {
        println!("also found: {}", others.join(", "));
    }
    if c11 {
        0
    } else {
        1
    }
}

fn cmd_explain(cli: &Cli) -> i32 {
    match cli.inputs.first() {
        None => {
            print!("{}", explain::list());
            0
        }
        Some(code) => match explain::lookup(code) {
            Some(e) => {
                print!("{}", explain::render(e));
                0
            }
            None => {
                eprintln!("keen: no explanation for `{}`; known codes:\n{}", code, explain::list());
                1
            }
        },
    }
}

fn cmd_completions(cli: &Cli) -> i32 {
    match cli.inputs.first().map(|s| s.as_str()) {
        Some(sh) => match completions::script(sh) {
            Some(s) => {
                print!("{}", s);
                0
            }
            None => usage_err(&format!("unknown shell `{}` (expected bash, zsh or fish)", sh)),
        },
        None => usage_err("`keen completions` needs a shell: bash, zsh or fish"),
    }
}

// ------------------------------------------------------------------------ watch

fn signature(dir: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn walk(dir: &Path, h: &mut DefaultHasher, depth: usize) {
        if depth > 12 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if p.is_dir() {
                if !(name.starts_with('.') || name == "target") {
                    walk(&p, h, depth + 1);
                }
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("kn" | "c" | "h" | "toml")) {
                if let Ok(md) = p.metadata() {
                    p.hash(h);
                    md.len().hash(h);
                    md.modified().ok().hash(h);
                }
            }
        }
    }
    let mut h = DefaultHasher::new();
    walk(dir, &mut h, 0);
    h.finish()
}

fn watch(cli: &Cli, proj: &Option<Proj>) -> i32 {
    let dir = match (proj, cli.inputs.iter().find(|i| ext_of(i) == "kn")) {
        (_, Some(f)) => Path::new(f).parent().map(|p| p.to_path_buf()).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from(".")),
        (Some(p), None) => p.root.clone(),
        (None, None) => PathBuf::from("."),
    };
    loop {
        let before = signature(&dir);
        let code = dispatch(cli, proj);
        eprintln!("{} exit code {}; watching {} for changes (Ctrl-C to stop)", paint("keen:", "1;36"), code, dir.display());
        while signature(&dir) == before {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        eprintln!("{} change detected", paint("keen:", "1;36"));
    }
}

// ---------------------------------------------------------------------- dispatch

fn dispatch(cli: &Cli, proj: &Option<Proj>) -> i32 {
    match cli.cmd {
        Cmd::Help => {
            print!("{}", cli::help(cli.inputs.first().map(|s| s.as_str())));
            0
        }
        Cmd::Version => {
            println!("keen {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Cmd::New => cmd_new(cli),
        Cmd::Init => cmd_init(cli),
        Cmd::Build => cmd_build(cli, proj),
        Cmd::Run => cmd_run(cli, proj),
        Cmd::Test => cmd_test(cli, proj),
        Cmd::Check => cmd_check(cli, proj),
        Cmd::Fmt => cmd_fmt(cli, proj),
        Cmd::Emit => cmd_emit(cli),
        Cmd::Clean => cmd_clean(cli, proj),
        Cmd::Doctor => cmd_doctor(cli),
        Cmd::Explain => cmd_explain(cli),
        Cmd::Completions => cmd_completions(cli),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = match cli::parse(args) {
        Ok(c) => c,
        Err(e) => exit(usage_err(&e)),
    };
    let color = match cli.color.as_str() {
        "always" => true,
        "never" => false,
        _ => {
            std::io::stderr().is_terminal()
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").map_or(true, |t| t != "dumb")
        }
    };
    COLOR.store(color, Ordering::Relaxed);
    let proj = match prepare(&mut cli) {
        Ok(p) => p,
        Err(e) => exit(err(&e)),
    };
    if cli.watch && matches!(cli.cmd, Cmd::Build | Cmd::Run | Cmd::Test | Cmd::Check | Cmd::Fmt) {
        exit(watch(&cli, &proj));
    }
    exit(dispatch(&cli, &proj));
}
