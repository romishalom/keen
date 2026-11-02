//! Build driver: find a C compiler, discover file modules, transpile, compile, run.
//!
//! keen itself never type-checks; the C compiler does. Because the generated C carries `#line`
//! directives, the compiler's diagnostics point at the original `.kn` files.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::ast::{Item, ItemKind};
use crate::emit::Options;
use crate::parser;

// ------------------------------------------------------------------ C compiler

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CcKind {
    Gcc,
    Clang,
    Other,
}

#[derive(Clone, Debug)]
pub struct Cc {
    pub program: String,
    pub args: Vec<String>,
    pub kind: CcKind,
    /// First line of `--version`.
    pub version: String,
}

impl Cc {
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        c
    }

    /// The command as the user would type it, e.g. `gcc -m32`.
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(|s| s.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            CcKind::Gcc => "gcc",
            CcKind::Clang => "clang",
            CcKind::Other => "cc",
        }
    }
}

/// Try to run `<spec> --version`; `spec` may contain arguments (`"gcc -m32"`).
fn probe(spec: &str) -> Option<Cc> {
    let mut parts = spec.split_whitespace();
    let program = parts.next()?.to_string();
    let args: Vec<String> = parts.map(String::from).collect();
    let out = Command::new(&program)
        .args(&args)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let lower = text.to_lowercase();
    let kind = if lower.contains("clang") {
        CcKind::Clang
    } else if lower.contains("gcc") || lower.contains("free software foundation") {
        CcKind::Gcc
    } else {
        CcKind::Other
    };
    let version = text.lines().next().unwrap_or("").trim().to_string();
    Some(Cc { program, args, kind, version })
}

/// Pick a compiler: `--cc`, then `$KEEN_CC`, then `$CC`, then `cc`, `gcc`, `clang`.
pub fn find_cc(explicit: Option<&str>) -> Result<Cc, String> {
    if let Some(spec) = explicit {
        return probe(spec)
            .ok_or_else(|| format!("C compiler `{}` could not be run (is it installed and on PATH?)", spec));
    }
    for var in ["KEEN_CC", "CC"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return probe(&v).ok_or_else(|| format!("${} is set to `{}`, which could not be run", var, v));
            }
        }
    }
    for cand in ["cc", "gcc", "clang"] {
        if let Some(cc) = probe(cand) {
            return Ok(cc);
        }
    }
    Err("no C compiler found (tried cc, gcc, clang). Install one, set KEEN_CC or CC, or pass --cc <compiler>"
        .to_string())
}

/// Does `cc` accept `snippet` with `-std=<std> -fsyntax-only`?
pub fn probe_std(cc: &Cc, std: &str, snippet: &str) -> bool {
    let dir = std::env::temp_dir().join(format!("keen-probe-{}", std::process::id()));
    if fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let file = dir.join("probe.c");
    let ok = fs::write(&file, snippet).is_ok()
        && cc
            .command()
            .arg(format!("-std={}", std))
            .arg("-fsyntax-only")
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    let _ = fs::remove_dir_all(&dir);
    ok
}

// --------------------------------------------------------------- module discovery

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub src: String,
    /// Module path of the file's top-level items (empty for the entry file).
    pub module: Vec<String>,
    /// Output stem: the entry file's stem, or the module name for file modules.
    pub stem: String,
    pub is_root: bool,
}

fn stem_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".to_string())
}

/// Collect `mod name;` declarations (module paths are relative to the declaring file).
fn file_mods(items: &[Item], prefix: &[String], out: &mut Vec<(Vec<String>, u32)>) {
    for it in items {
        if let ItemKind::Mod { name, items } = &it.kind {
            let mut p = prefix.to_vec();
            p.push(name.clone());
            match items {
                Some(sub) => file_mods(sub, &p, out),
                None => out.push((p, it.line)),
            }
        }
    }
}

/// `dir/name.kn` or `dir/name/mod.kn`, looking first in the declaring file's child directory.
fn module_candidates(parent: &SourceFile, name: &str) -> Vec<PathBuf> {
    let dir = parent.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let is_mod_file = parent.path.file_name().map(|f| f == "mod.kn").unwrap_or(false);
    let mut dirs = Vec::new();
    if parent.is_root || is_mod_file {
        dirs.push(dir);
    } else {
        dirs.push(dir.join(&parent.stem));
        dirs.push(dir);
    }
    let mut out = Vec::new();
    for d in dirs {
        out.push(d.join(format!("{}.kn", name)));
        out.push(d.join(name).join("mod.kn"));
    }
    out
}

fn canon(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Read the entry file and every file module reachable through `mod name;` declarations.
pub fn discover(entry: &Path) -> Result<Vec<SourceFile>, String> {
    let src = fs::read_to_string(entry).map_err(|e| format!("cannot read `{}`: {}", entry.display(), e))?;
    let root_stem = stem_of(entry);
    let mut files = vec![SourceFile {
        path: entry.to_path_buf(),
        src,
        module: Vec::new(),
        stem: root_stem.clone(),
        is_root: true,
    }];
    let mut by_name: HashMap<String, usize> = HashMap::new();
    by_name.insert(root_stem, 0);

    let mut i = 0;
    while i < files.len() {
        let parent = files[i].clone();
        let shown = parent.path.display().to_string();
        let items = parser::parse(&parent.src).map_err(|d| d.render_with_source(&shown, &parent.src))?;
        let mut mods = Vec::new();
        file_mods(&items, &[], &mut mods);
        for (rel, line) in mods {
            let name = rel.last().cloned().unwrap_or_default();
            let mut modpath = parent.module.clone();
            modpath.extend(rel.iter().cloned());
            let cands = module_candidates(&parent, &name);
            let Some(found) = cands.iter().find(|c| c.is_file()).cloned() else {
                let looked: Vec<String> = cands.iter().map(|c| c.display().to_string()).collect();
                return Err(format!(
                    "{}:{}: error: file for `mod {};` not found (looked for: {})",
                    shown,
                    line,
                    name,
                    looked.join(", ")
                ));
            };
            if let Some(&j) = by_name.get(&name) {
                if canon(&files[j].path) != canon(&found) {
                    return Err(format!(
                        "{}:{}: error: two different files are named module `{}` ({} and {}); module names must be unique because they become header names",
                        shown,
                        line,
                        name,
                        files[j].path.display(),
                        found.display()
                    ));
                }
                if files[j].module != modpath {
                    return Err(format!(
                        "{}:{}: error: module `{}` is declared at two different paths (`{}` and `{}`)",
                        shown,
                        line,
                        name,
                        files[j].module.join("::"),
                        modpath.join("::")
                    ));
                }
                continue;
            }
            let src = fs::read_to_string(&found)
                .map_err(|e| format!("cannot read `{}`: {}", found.display(), e))?;
            by_name.insert(name.clone(), files.len());
            files.push(SourceFile { path: found, src, module: modpath, stem: name, is_root: false });
        }
        i += 1;
    }
    Ok(files)
}

/// Transpile every discovered file into `out_dir`. The entry file becomes a single `.c`;
/// file modules become a `.h`/`.c` pair. Returns the `.c` files, entry first.
pub fn transpile_files(
    files: &[SourceFile],
    base: &Options,
    out_dir: &Path,
    verbose: bool,
) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("cannot create `{}`: {}", out_dir.display(), e))?;
    let mut c_files = Vec::new();
    for f in files {
        let mut o = base.clone();
        o.file = f.path.display().to_string();
        o.stem = f.stem.clone();
        if f.is_root {
            o.split = false;
            o.module = None;
            o.test_modules = files.iter().filter(|x| !x.is_root).map(|x| x.stem.clone()).collect();
        } else {
            o.split = true;
            o.module = Some(f.module.join("::"));
        }
        let out = crate::transpile(&f.src, &o).map_err(|d| d.render_with_source(&o.file, &f.src))?;
        let cpath = out_dir.join(format!("{}.c", f.stem));
        fs::write(&cpath, &out.c).map_err(|e| format!("cannot write `{}`: {}", cpath.display(), e))?;
        if let Some(h) = &out.h {
            let hpath = out_dir.join(format!("{}.h", f.stem));
            fs::write(&hpath, h).map_err(|e| format!("cannot write `{}`: {}", hpath.display(), e))?;
        }
        if verbose {
            eprintln!("keen: {} -> {}", f.path.display(), cpath.display());
        }
        c_files.push(cpath);
    }
    Ok(c_files)
}

// --------------------------------------------------------------------- compiling

#[derive(Clone, Debug)]
pub struct CcOptions {
    pub cc: Option<String>,
    pub release: bool,
    /// Explicit optimization level (`0`, `1`, `2`, `3`, `s`, `g`, `fast`).
    pub opt: Option<String>,
    pub debug: bool,
    pub warnings: bool,
    pub cflags: Vec<String>,
    pub ldflags: Vec<String>,
    /// Extra `.c`/`.o`/`.a` files to link in.
    pub extra_inputs: Vec<PathBuf>,
    pub verbose: bool,
    /// Skip compiling when nothing changed since the last successful build in the same out dir.
    pub incremental: bool,
}

impl Default for CcOptions {
    fn default() -> Self {
        CcOptions {
            cc: None,
            release: false,
            opt: None,
            debug: false,
            warnings: true,
            cflags: Vec::new(),
            ldflags: Vec::new(),
            extra_inputs: Vec::new(),
            verbose: false,
            incremental: false,
        }
    }
}

fn base_flags(cc: &Cc, o: &CcOptions, std23: bool, include_dirs: &[PathBuf]) -> Vec<String> {
    // GNU modes: the generated code uses `__auto_type`, case ranges and `__attribute__`.
    let mut f = vec![format!("-std={}", if std23 { "gnu2x" } else { "gnu11" })];
    match &o.opt {
        Some(level) => f.push(format!("-O{}", level)),
        None => f.push(if o.release { "-O2" } else { "-O0" }.to_string()),
    }
    if o.debug || !o.release {
        f.push("-g".to_string());
    }
    if o.warnings {
        f.push("-Wall".to_string());
        f.push("-Wextra".to_string());
        f.push("-Wno-unused-parameter".to_string());
        f.push(
            if cc.kind == CcKind::Clang { "-Wno-unused-local-typedef" } else { "-Wno-unused-local-typedefs" }
                .to_string(),
        );
    } else {
        f.push("-w".to_string());
    }
    for d in include_dirs {
        let d = if d.as_os_str().is_empty() { Path::new(".") } else { d.as_path() };
        f.push(format!("-I{}", d.display()));
    }
    f.extend(o.cflags.iter().cloned());
    f
}

fn describe_status(s: ExitStatus) -> String {
    match s.code() {
        Some(c) => format!("exit code {}", c),
        None => "terminated by a signal".to_string(),
    }
}

fn fmt_cmd(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|s| s.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_cmd(cmd: &mut Command, verbose: bool, what: &str) -> Result<(), String> {
    if verbose {
        eprintln!("+ {}", fmt_cmd(cmd));
    }
    let status = cmd.status().map_err(|e| format!("failed to start {}: {}", what, e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({})", what, describe_status(status)))
    }
}

fn include_dirs(entry: &Path, out_dir: &Path) -> Vec<PathBuf> {
    vec![out_dir.to_path_buf(), entry.parent().map(|p| p.to_path_buf()).unwrap_or_default()]
}

/// A hash of everything that influences the build (sources, flags, compiler, headers, C inputs).
fn fingerprint(
    files: &[SourceFile],
    cc: &Cc,
    flags: &[String],
    topts: &Options,
    extra: &[PathBuf],
    entry: &Path,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for f in files {
        f.path.hash(&mut h);
        f.src.hash(&mut h);
        f.module.hash(&mut h);
    }
    cc.display().hash(&mut h);
    cc.version.hash(&mut h);
    flags.hash(&mut h);
    (topts.std23, topts.minimal_parens, topts.line_directives, topts.case_ranges, topts.test).hash(&mut h);
    topts.sep.hash(&mut h);
    for e in extra {
        e.hash(&mut h);
        fs::read(e).unwrap_or_default().hash(&mut h);
    }
    // user headers next to the entry file may be `use "x.h";`-included
    if let Ok(rd) = fs::read_dir(entry.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))) {
        let mut hs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "h"))
            .collect();
        hs.sort();
        for p in hs.into_iter().take(200) {
            p.hash(&mut h);
            fs::read(&p).unwrap_or_default().hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

/// Result of a build: the compiler used and whether the existing binary was already up to date.
pub struct Built {
    pub cc: Cc,
    pub up_to_date: bool,
}

/// Transpile `entry` (and its file modules) and compile everything into `output`.
pub fn build(
    entry: &Path,
    topts: &Options,
    o: &CcOptions,
    out_dir: &Path,
    output: &Path,
) -> Result<Built, String> {
    let cc = find_cc(o.cc.as_deref())?;
    if o.verbose {
        eprintln!("keen: using {} ({})", cc.display(), cc.version);
    }
    let files = discover(entry)?;
    let flags = base_flags(&cc, o, topts.std23, &include_dirs(entry, out_dir));
    let print = fingerprint(&files, &cc, &flags, topts, &o.extra_inputs, entry);
    let print_file = out_dir.join(".fingerprint");
    if o.incremental && output.exists() && fs::read_to_string(&print_file).map(|s| s == print).unwrap_or(false) {
        return Ok(Built { cc, up_to_date: true });
    }
    let c_files = transpile_files(&files, topts, out_dir, o.verbose)?;
    let mut cmd = cc.command();
    cmd.args(&flags);
    cmd.args(&c_files);
    cmd.args(&o.extra_inputs);
    cmd.arg("-o").arg(output);
    cmd.args(&o.ldflags);
    let _ = fs::remove_file(&print_file);
    run_cmd(&mut cmd, o.verbose, &format!("the C compiler ({})", cc.kind_name()))?;
    if o.incremental {
        let _ = fs::write(&print_file, &print);
    }
    Ok(Built { cc, up_to_date: false })
}

/// Transpile and ask the C compiler to check the result without producing a binary.
pub fn check(entry: &Path, topts: &Options, o: &CcOptions, out_dir: &Path) -> Result<Cc, String> {
    let cc = find_cc(o.cc.as_deref())?;
    if o.verbose {
        eprintln!("keen: using {} ({})", cc.display(), cc.version);
    }
    let files = discover(entry)?;
    let c_files = transpile_files(&files, topts, out_dir, o.verbose)?;
    for c in &c_files {
        let mut cmd = cc.command();
        cmd.args(base_flags(&cc, o, topts.std23, &include_dirs(entry, out_dir)));
        cmd.arg("-fsyntax-only").arg(c);
        run_cmd(&mut cmd, o.verbose, &format!("the C compiler ({})", cc.kind_name()))?;
    }
    Ok(cc)
}

/// Run a built program, inheriting stdio. Returns its exit code (128+signal when killed).
pub fn run_program(exe: &Path, args: &[String], verbose: bool) -> Result<i32, String> {
    let exe = if exe.is_relative() && exe.components().count() == 1 {
        Path::new(".").join(exe)
    } else {
        exe.to_path_buf()
    };
    if verbose {
        eprintln!("+ {} {}", exe.display(), args.join(" "));
    }
    let status = Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run `{}`: {}", exe.display(), e))?;
    if let Some(c) = status.code() {
        return Ok(c);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Ok(128 + sig);
        }
    }
    Ok(1)
}
