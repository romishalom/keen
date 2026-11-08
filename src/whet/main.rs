//! whet — the keen toolchain installer and manager (think rustup, for keen).
//!
//! Layout of `$KEEN_HOME` (default `~/.keen`):
//!
//! ```text
//! bin/keen        symlink to the default toolchain's keen
//! bin/whet        this program
//! toolchains/<v>/ one directory per installed version
//! env             sourced by shell profiles to put bin/ on PATH
//! settings.toml   what whet did (default toolchain, edited files) so it can undo it
//! ```

mod net;
mod platform;
mod settings;
mod sha256;
mod shell;
mod toolchain;
mod ui;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use platform::Version;
use settings::Settings;
use toolchain::Ctx;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_DIST: &str = match option_env!("WHET_DEFAULT_DIST_SERVER") {
    Some(s) => s,
    None => "https://github.com/romishalom/keen/releases",
};

const OVERVIEW: &str = "\
whet - the keen toolchain installer and manager

USAGE:
    whet <command> [options]

COMMANDS:
    init                     Install keen (interactive; `-y` accepts the defaults)
    update [--check]         Install the newest keen and make it the default
    install <version>        Install a toolchain (`latest` or e.g. 0.2.1)
    default <version>        Choose which installed toolchain `keen` runs
    toolchain list           List installed toolchains
    toolchain install <v>    Same as `whet install`
    toolchain uninstall <v>  Remove a toolchain
    show                     Show the installation, toolchains and C compiler
    which                    Print the path of the default `keen`
    run <version> <cmd> ...  Run a command with a specific toolchain first on PATH
    check [--online]         Diagnose problems (PATH, links, C compiler, dist server)
    self update              Update whet itself
    self uninstall           Remove keen, whet and the shell profile changes
    completions <shell>      Print completions (bash, zsh, fish)
    help [command]           Show help

Run `whet help init` for the installer's advanced options.
";

const INIT_HELP: &str = "\
whet init - install keen

USAGE:
    whet init [options]

With no options (and a terminal) you get the standard installation menu: press Enter to accept
the defaults, or choose \"Customize installation\".

OPTIONS:
    -y, --yes                      Do not prompt; use the defaults and any options below
    --default-toolchain <v>        latest (default), a version such as 0.2.1, or `none`
                                   (`none` installs only whet itself)
    --profile <p>                  default (keen + shell completions) or minimal (keen only)
    --no-modify-path               Do not edit shell profiles (an `env` file is still written)
    --prefix <dir>                 Install root (default: $KEEN_HOME, else ~/.keen)
    --dist-server <url>            Where releases come from (default: $WHET_DIST_SERVER, else the
                                   official server). https://, file:// and loopback http:// work.
    --target <triple>              Override the detected platform (cross-installs, testing)
    --force                        Reinstall even if the toolchain is already present
    --no-cc-check                  Do not warn when no C compiler is found
    -q, --quiet / -v, --verbose

ENVIRONMENT:
    KEEN_HOME  WHET_DIST_SERVER  WHET_TARGET  WHET_ALLOW_HTTP  NO_COLOR

EXAMPLES:
    whet init -y --default-toolchain 0.2.1 --profile minimal --no-modify-path
    whet init -y --prefix /opt/keen --dist-server file:///mnt/mirror/dist
";

#[derive(Default, Clone)]
struct Opts {
    yes: bool,
    toolchain: Option<String>,
    profile: Option<String>,
    no_modify_path: bool,
    home: Option<String>,
    dist: Option<String>,
    target: Option<String>,
    force: bool,
    no_cc_check: bool,
    check_only: bool,
    online: bool,
    quiet: bool,
    verbose: bool,
    pos: Vec<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    let val = |it: &mut std::slice::Iter<String>, f: &str| -> Result<String, String> {
        it.next().cloned().ok_or_else(|| format!("`{}` needs a value", f))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-y" | "--yes" => o.yes = true,
            "--default-toolchain" => o.toolchain = Some(val(&mut it, a)?),
            "--profile" => {
                let p = val(&mut it, a)?;
                if p != "default" && p != "minimal" {
                    return Err("`--profile` takes `default` or `minimal`".into());
                }
                o.profile = Some(p);
            }
            "--no-modify-path" => o.no_modify_path = true,
            "--prefix" | "--home" => o.home = Some(val(&mut it, a)?),
            "--dist-server" => o.dist = Some(val(&mut it, a)?),
            "--target" => o.target = Some(val(&mut it, a)?),
            "--force" => o.force = true,
            "--no-cc-check" => o.no_cc_check = true,
            "--check" => o.check_only = true,
            "--online" => o.online = true,
            "-q" | "--quiet" => o.quiet = true,
            "-v" | "--verbose" => o.verbose = true,
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option `{}`", s)),
            _ => o.pos.push(a.clone()),
        }
    }
    Ok(o)
}

// ------------------------------------------------------------------- context

fn home_dir(o: &Opts) -> PathBuf {
    if let Some(h) = &o.home {
        return absolute(Path::new(h));
    }
    if let Some(h) = std::env::var_os("KEEN_HOME").filter(|h| !h.is_empty()) {
        return absolute(Path::new(&h));
    }
    shell::user_home().join(".keen")
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

fn dist_server(o: &Opts, s: &Settings) -> String {
    o.dist
        .clone()
        .or_else(|| std::env::var("WHET_DIST_SERVER").ok().filter(|v| !v.is_empty()))
        .or_else(|| s.dist_server.clone())
        .unwrap_or_else(|| DEFAULT_DIST.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn make_ctx(o: &Opts) -> (Ctx, Settings) {
    let home = home_dir(o);
    let s = settings::load(&home);
    let ctx = Ctx {
        dist: dist_server(o, &s),
        target: o.target.clone().unwrap_or_else(platform::target),
        home,
    };
    (ctx, s)
}

fn pretty(p: &Path) -> String {
    let uh = shell::user_home();
    match p.strip_prefix(&uh) {
        Ok(rest) if !uh.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

fn fail(msg: &str) -> i32 {
    ui::error(msg);
    1
}

fn usage(msg: &str) -> i32 {
    ui::error(msg);
    eprintln!("try `whet help`");
    2
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

fn find_in_path(name: &str) -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else { return Vec::new() };
    std::env::split_paths(&path)
        .map(|d| d.join(Ctx::exe(name)))
        .filter(|p| p.is_file())
        .collect()
}

// ------------------------------------------------------------------- init

struct Plan {
    home: PathBuf,
    toolchain: String, // latest | x.y.z | none
    profile: String,
    modify_path: bool,
    dist: String,
}

fn print_plan(p: &Plan) {
    println!("Current installation options:\n");
    println!("   {:<20} {}", "install dir:", pretty(&p.home));
    println!("   {:<20} {}", "default toolchain:", if p.toolchain == "latest" { "latest (resolved at install time)".to_string() } else { p.toolchain.clone() });
    println!("   {:<20} {}", "profile:", p.profile);
    println!("   {:<20} {}", "modify PATH:", if p.modify_path { "yes" } else { "no" });
    println!("   {:<20} {}", "dist server:", p.dist);
    println!();
}

/// The standard-vs-customize menu. Returns `None` if the user cancels.
fn interactive(mut plan: Plan) -> Option<Plan> {
    println!("{}", ui::bold("Welcome to keen!"));
    println!();
    println!("This will download and install keen, a Rust-syntax front end for C, together with");
    println!("`whet`, the tool that manages it. Everything lives in {}", pretty(&plan.home));
    println!("and nothing outside it is touched except your shell profile (to add keen to PATH).");
    println!();
    loop {
        print_plan(&plan);
        println!("1) Proceed with standard installation (default - just press enter)");
        println!("2) Customize installation");
        println!("3) Cancel installation");
        let choice = ui::ask_plain(">", "1");
        println!();
        match choice.as_str() {
            "1" => return Some(plan),
            "3" => return None,
            "2" => {
                let t = ui::ask("Default toolchain (latest, a version like 0.2.1, or none)", &plan.toolchain);
                if t == "latest" || t == "none" || platform::valid_version(t.trim_start_matches('v')) {
                    plan.toolchain = t.trim_start_matches('v').to_string();
                } else {
                    ui::warn(&format!("`{}` is not a valid choice; keeping {}", t, plan.toolchain));
                }
                let p = ui::ask("Profile (default = keen + completions, minimal = keen only)", &plan.profile);
                if p == "default" || p == "minimal" {
                    plan.profile = p;
                } else {
                    ui::warn(&format!("`{}` is not a valid profile; keeping {}", p, plan.profile));
                }
                plan.modify_path = ui::ask_yes_no("Modify the PATH variable in your shell profiles?", plan.modify_path);
                let h = ui::ask("Install directory", &plan.home.display().to_string());
                plan.home = absolute(Path::new(&h));
                plan.dist = ui::ask("Dist server", &plan.dist).trim_end_matches('/').to_string();
                println!();
            }
            other => println!("please choose 1, 2 or 3 (got `{}`)\n", other),
        }
    }
}

fn install_self(ctx: &Ctx) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("cannot locate the running whet: {}", e))?;
    let me = std::fs::canonicalize(&me).unwrap_or(me);
    let dest = ctx.bin_dir().join(Ctx::exe("whet"));
    if std::fs::canonicalize(&dest).ok().as_deref() == Some(me.as_path()) {
        return Ok(());
    }
    std::fs::create_dir_all(ctx.bin_dir()).map_err(|e| format!("cannot create `{}`: {}", ctx.bin_dir().display(), e))?;
    let tmp = ctx.bin_dir().join(".whet.new");
    std::fs::copy(&me, &tmp).map_err(|e| format!("cannot copy whet into `{}`: {}", ctx.bin_dir().display(), e))?;
    toolchain::make_executable(&tmp);
    std::fs::rename(&tmp, &dest).map_err(|e| format!("cannot install whet: {}", e))
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("cannot create `{}`: {}", p.display(), e))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write `{}`: {}", path.display(), e))
}

/// Generate keen's own completion scripts with the installed toolchain.
fn install_completions(ctx: &Ctx, version: &str, s: &mut Settings) {
    let keen = ctx.toolchain_dir(version).join("bin").join(Ctx::exe("keen"));
    let uh = shell::user_home();
    let shell_name = std::env::var("SHELL").unwrap_or_default();
    let mut targets: Vec<(&str, PathBuf, bool)> = vec![
        ("bash", ctx.home.join("share/completions/keen.bash"), false),
        ("zsh", ctx.home.join("share/zsh/site-functions/_keen"), false),
    ];
    if shell::uses_fish(&uh, &shell_name) {
        targets.push(("fish", shell::fish_dir(&uh).join("completions/keen.fish"), true));
    }
    for (sh, path, outside) in targets {
        let out = Command::new(&keen).args(["completions", sh]).output();
        match out {
            Ok(o) if o.status.success() => {
                if write_file(&path, &String::from_utf8_lossy(&o.stdout)).is_ok() && outside {
                    push_unique(&mut s.created, path.display().to_string());
                }
            }
            _ => ui::debug(&format!("could not generate {} completions", sh)),
        }
    }
}

fn modify_path(ctx: &Ctx, s: &mut Settings) -> Result<(), String> {
    let uh = shell::user_home();
    let shell_name = std::env::var("SHELL").unwrap_or_default();
    let hexpr = shell::home_expr(&ctx.home, &uh);
    if !cfg!(unix) {
        ui::warn(&format!("add {} to your PATH manually (automatic PATH setup is Unix-only)", ctx.bin_dir().display()));
        return Ok(());
    }
    let block = shell::source_block(&hexpr);
    for f in shell::sh_profiles(&uh, &shell_name) {
        shell::add_block(&f, &block)?;
        push_unique(&mut s.touched, f.display().to_string());
        ui::debug(&format!("updated {}", f.display()));
    }
    if shell::uses_fish(&uh, &shell_name) {
        let conf = shell::fish_dir(&uh).join("conf.d/keen.fish");
        write_file(&conf, &shell::fish_script(&ctx.bin_dir().to_string_lossy()))?;
        push_unique(&mut s.created, conf.display().to_string());
    }
    Ok(())
}

fn cmd_init(o: &Opts) -> i32 {
    let mut plan = Plan {
        home: home_dir(o),
        toolchain: o.toolchain.clone().unwrap_or_else(|| "latest".to_string()).trim_start_matches('v').to_string(),
        profile: o.profile.clone().unwrap_or_else(|| "default".to_string()),
        modify_path: !o.no_modify_path,
        dist: dist_server(o, &settings::load(&home_dir(o))),
    };
    if plan.toolchain != "latest" && plan.toolchain != "none" && !platform::valid_version(&plan.toolchain) {
        return usage(&format!("`{}` is not a version (use `latest`, `none` or something like 0.2.1)", plan.toolchain));
    }
    if !o.yes {
        if !ui::can_prompt() {
            return fail("no terminal is available for prompts; re-run with `-y` to accept the defaults (see `whet help init`)");
        }
        match interactive(plan) {
            Some(p) => plan = p,
            None => {
                println!("Installation cancelled.");
                return 0;
            }
        }
        if settings::exists(&plan.home) && !o.force {
            let s = settings::load(&plan.home);
            let have = s.default_toolchain.unwrap_or_else(|| "?".into());
            if !ui::ask_yes_no(&format!("keen {} is already installed in {}. Install over it?", have, pretty(&plan.home)), false) {
                println!("Installation cancelled.");
                return 0;
            }
        }
    }

    let existing = settings::load(&plan.home);
    let ctx = Ctx {
        home: plan.home.clone(),
        dist: plan.dist.clone(),
        target: o.target.clone().unwrap_or_else(platform::target),
    };
    let mut s = existing;
    match run_init(&ctx, &plan, o, &mut s) {
        Ok(version) => {
            print_success(&ctx, &plan, version.as_deref());
            0
        }
        Err(e) => fail(&e),
    }
}

fn run_init(ctx: &Ctx, plan: &Plan, o: &Opts, s: &mut Settings) -> Result<Option<String>, String> {
    for d in [ctx.bin_dir(), ctx.toolchains_dir(), ctx.tmp_dir()] {
        std::fs::create_dir_all(&d).map_err(|e| format!("cannot create `{}`: {}", d.display(), e))?;
    }
    // Resolve and install first, so a failed download leaves nothing half-configured.
    let version = if plan.toolchain == "none" {
        None
    } else {
        let v = toolchain::resolve(ctx, &plan.toolchain)?;
        let fresh = toolchain::install(ctx, &v, o.force)?;
        if !fresh {
            ui::info(&format!("toolchain {} is already installed", v));
        }
        Some(v)
    };
    install_self(ctx)?;
    if let Some(v) = &version {
        toolchain::set_default(ctx, v)?;
        s.default_toolchain = Some(v.clone());
    }
    s.profile = plan.profile.clone();
    s.dist_server = Some(ctx.dist.clone());
    let uh = shell::user_home();
    write_file(&ctx.home.join("env"), &shell::env_script(&shell::home_expr(&ctx.home, &uh)))?;
    if plan.modify_path {
        modify_path(ctx, s)?;
    }
    if plan.profile == "default" {
        if let Some(v) = &version {
            install_completions(ctx, v, s);
        }
    }
    settings::save(&ctx.home, s)?;
    Ok(version)
}

fn print_success(ctx: &Ctx, plan: &Plan, version: Option<&str>) {
    println!();
    match version {
        Some(v) => println!("{}", ui::paint(&format!("keen {} is installed now. Great!", v), "1;32")),
        None => println!("{}", ui::paint("whet is installed now (no toolchain yet; run `whet install latest`).", "1;32")),
    }
    println!();
    if !ui::quiet() {
        let bin = pretty(&ctx.bin_dir());
        if plan.modify_path && cfg!(unix) {
            println!("To get started you may need to restart your shell so PATH picks up {}.", bin);
            println!("To use keen in this shell right now, run:");
        } else {
            println!("keen's bin directory is {} (your PATH was not modified).", bin);
            println!("To use keen in a shell, run:");
        }
        println!("    . \"{}/env\"", shell::home_expr(&ctx.home, &shell::user_home()).replace("$HOME", "$HOME"));
        println!();
        if version.is_some() {
            println!("Then try:  keen new hello && cd hello && keen run");
        }
        let others: Vec<PathBuf> = find_in_path("keen").into_iter().filter(|p| !p.starts_with(ctx.bin_dir())).collect();
        if !others.is_empty() {
            ui::warn(&format!(
                "another `keen` is already on your PATH ({}); make sure {} comes first",
                others[0].display(),
                pretty(&ctx.bin_dir())
            ));
        }
    }
    if version.is_some() && toolchain::find_cc().is_none() {
        ui::warn("no C compiler (cc, gcc or clang) was found; keen needs one to build programs");
        ui::warn(&format!("install one: {}", toolchain::cc_install_hint()));
    }
}

// ---------------------------------------------------------------- commands

fn need_install(ctx: &Ctx, s: &Settings) -> Result<(), String> {
    if !settings::exists(&ctx.home) {
        return Err(format!("keen is not installed in {} (run `whet init`)", pretty(&ctx.home)));
    }
    let _ = s;
    Ok(())
}

fn cmd_update(o: &Opts) -> i32 {
    let (ctx, mut s) = make_ctx(o);
    if let Err(e) = need_install(&ctx, &s) {
        return fail(&e);
    }
    let latest = match toolchain::resolve(&ctx, "latest") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let current = s.default_toolchain.clone();
    let newer = match current.as_deref().and_then(Version::parse) {
        Some(c) => Version::parse(&latest).map_or(false, |l| l > c),
        None => true,
    };
    if o.check_only {
        match &current {
            Some(c) if !newer => println!("keen {} is up to date", c),
            Some(c) => println!("update available: {} -> {}", c, latest),
            None => println!("no toolchain installed; latest is {}", latest),
        }
        return 0;
    }
    if !newer && !o.force {
        match &current {
            Some(c) if *c == latest => ui::info(&format!("keen {} is up to date", c)),
            Some(c) => ui::info(&format!("the installed keen {} is newer than the latest release {}", c, latest)),
            None => {}
        }
        return 0;
    }
    if let Err(e) = toolchain::install(&ctx, &latest, o.force).and_then(|_| toolchain::set_default(&ctx, &latest)) {
        return fail(&e);
    }
    s.default_toolchain = Some(latest.clone());
    if s.profile == "default" {
        install_completions(&ctx, &latest, &mut s);
    }
    if let Err(e) = settings::save(&ctx.home, &s) {
        return fail(&e);
    }
    match current {
        Some(c) if c != latest => println!("updated keen {} -> {}", c, latest),
        _ => println!("installed keen {}", latest),
    }
    0
}

fn cmd_install(o: &Opts, spec: Option<&String>) -> i32 {
    let Some(spec) = spec else { return usage("`whet install` needs a version (e.g. `latest` or 0.2.1)") };
    let (ctx, mut s) = make_ctx(o);
    if let Err(e) = need_install(&ctx, &s) {
        return fail(&e);
    }
    let v = match toolchain::resolve(&ctx, spec) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    match toolchain::install(&ctx, &v, o.force) {
        Ok(true) => println!("installed toolchain {}", v),
        Ok(false) => println!("toolchain {} is already installed", v),
        Err(e) => return fail(&e),
    }
    if s.default_toolchain.is_none() {
        if let Err(e) = toolchain::set_default(&ctx, &v) {
            return fail(&e);
        }
        s.default_toolchain = Some(v.clone());
        let _ = settings::save(&ctx.home, &s);
        println!("{} is now the default toolchain", v);
    }
    0
}

fn cmd_default(o: &Opts, spec: Option<&String>) -> i32 {
    let Some(spec) = spec else { return usage("`whet default` needs a version") };
    let (ctx, mut s) = make_ctx(o);
    if let Err(e) = need_install(&ctx, &s) {
        return fail(&e);
    }
    let v = spec.trim_start_matches('v').to_string();
    if let Err(e) = toolchain::set_default(&ctx, &v) {
        return fail(&e);
    }
    s.default_toolchain = Some(v.clone());
    if let Err(e) = settings::save(&ctx.home, &s) {
        return fail(&e);
    }
    println!("default toolchain is now {}", v);
    0
}

fn cmd_toolchain(o: &Opts) -> i32 {
    let sub = o.pos.first().map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" => {
            let (ctx, s) = make_ctx(o);
            for v in toolchain::list(&ctx) {
                if s.default_toolchain.as_deref() == Some(v.as_str()) {
                    println!("{} (default)", v);
                } else {
                    println!("{}", v);
                }
            }
            0
        }
        "install" => cmd_install(o, o.pos.get(1)),
        "uninstall" | "remove" => {
            let Some(v) = o.pos.get(1) else { return usage("`whet toolchain uninstall` needs a version") };
            let (ctx, s) = make_ctx(o);
            match toolchain::uninstall(&ctx, v.trim_start_matches('v'), s.default_toolchain.as_deref()) {
                Ok(()) => {
                    println!("removed toolchain {}", v);
                    0
                }
                Err(e) => fail(&e),
            }
        }
        other => usage(&format!("unknown toolchain command `{}` (list, install, uninstall)", other)),
    }
}

fn cmd_show(o: &Opts) -> i32 {
    let (ctx, s) = make_ctx(o);
    println!("whet {} ({})", VERSION, ctx.target);
    println!("home:         {}", pretty(&ctx.home));
    println!("dist server:  {}", ctx.dist);
    if !settings::exists(&ctx.home) {
        println!("status:       not installed (run `whet init`)");
        return 0;
    }
    println!("profile:      {}", s.profile);
    println!("default:      {}", s.default_toolchain.as_deref().unwrap_or("(none)"));
    println!("installed:");
    let list = toolchain::list(&ctx);
    if list.is_empty() {
        println!("  (none)");
    }
    for v in list {
        println!("  {}{}", v, if s.default_toolchain.as_deref() == Some(v.as_str()) { " (default)" } else { "" });
    }
    match toolchain::find_cc() {
        Some(c) => println!("C compiler:   {}", c),
        None => println!("C compiler:   not found ({})", toolchain::cc_install_hint()),
    }
    0
}

fn cmd_which(o: &Opts) -> i32 {
    let (ctx, s) = make_ctx(o);
    match s.default_toolchain {
        Some(v) => {
            println!("{}", ctx.toolchain_dir(&v).join("bin").join(Ctx::exe("keen")).display());
            0
        }
        None => fail("no default toolchain (run `whet install latest`)"),
    }
}

fn cmd_run(o: &Opts, raw: &[String]) -> i32 {
    // `whet run <version> <command> [args...]`; everything after the version is the command.
    let rest: Vec<&String> = raw.iter().collect();
    if rest.len() < 2 {
        return usage("usage: whet run <version> <command> [args...]");
    }
    let (ctx, _) = make_ctx(o);
    let v = rest[0].trim_start_matches('v');
    let bin = ctx.toolchain_dir(v).join("bin");
    if !bin.join(Ctx::exe("keen")).exists() {
        return fail(&format!("toolchain {} is not installed (try `whet install {}`)", v, v));
    }
    let mut path = std::ffi::OsString::from(&bin);
    if let Some(old) = std::env::var_os("PATH") {
        path.push(if cfg!(windows) { ";" } else { ":" });
        path.push(old);
    }
    let cmd = rest[1];
    let program = if bin.join(Ctx::exe(cmd)).exists() { bin.join(Ctx::exe(cmd)) } else { PathBuf::from(cmd) };
    match Command::new(program).args(&rest[2..]).env("PATH", path).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => fail(&format!("cannot run `{}`: {}", cmd, e)),
    }
}

fn cmd_check(o: &Opts) -> i32 {
    let (ctx, s) = make_ctx(o);
    let mut problems = 0;
    let mut line = |ok: bool, what: String, fixit: Option<String>| {
        println!("{} {}", if ok { ui::paint("ok  ", "1;32") } else { ui::paint("FAIL", "1;31") }, what);
        if !ok {
            problems += 1;
            if let Some(f) = fixit {
                println!("       {}", f);
            }
        }
    };
    let installed = settings::exists(&ctx.home);
    line(installed, format!("whet home {}", pretty(&ctx.home)), Some("run `whet init`".into()));
    if installed {
        let def = s.default_toolchain.clone();
        line(def.is_some(), "default toolchain is set".into(), Some("run `whet install latest`".into()));
        if let Some(d) = &def {
            let link = ctx.bin_dir().join(Ctx::exe("keen"));
            let works = Command::new(&link).arg("--version").output().map(|r| r.status.success()).unwrap_or(false);
            line(works, format!("{} runs (keen {})", pretty(&link), d), Some(format!("run `whet default {}`", d)));
        }
        let on_path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d == ctx.bin_dir()))
            .unwrap_or(false);
        line(on_path, format!("{} is on PATH", pretty(&ctx.bin_dir())), Some(format!("add `. \"{}/env\"` to your shell profile", pretty(&ctx.home))));
    }
    let cc = toolchain::find_cc();
    line(cc.is_some(), format!("C compiler: {}", cc.clone().unwrap_or_else(|| "none found".into())), Some(toolchain::cc_install_hint().to_string()));
    if o.online {
        let r = toolchain::resolve(&ctx, "latest");
        line(r.is_ok(), format!("dist server {} ({})", ctx.dist, r.clone().map(|v| format!("latest is {}", v)).unwrap_or_else(|e| e)), None);
    }
    if problems == 0 {
        0
    } else {
        1
    }
}

fn cmd_self(o: &Opts) -> i32 {
    match o.pos.first().map(|s| s.as_str()) {
        Some("update") => self_update(o),
        Some("uninstall") => self_uninstall(o),
        _ => usage("usage: whet self <update|uninstall>"),
    }
}

fn self_update(o: &Opts) -> i32 {
    let (ctx, _) = make_ctx(o);
    let latest = match toolchain::resolve(&ctx, "latest") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let newer = match (Version::parse(&latest), Version::parse(VERSION)) {
        (Some(l), Some(c)) => l > c,
        _ => true,
    };
    if !newer && !o.force {
        ui::info(&format!("whet {} is up to date", VERSION));
        return 0;
    }
    let file = Ctx::exe(&format!("whet-{}", ctx.target));
    // GitHub releases flat asset download path
    let url = format!("{}/download/v{}/{}", ctx.dist.trim_end_matches('/'), latest, file);
    let tmp = ctx.bin_dir().join(".whet.update");
    let r = std::fs::create_dir_all(ctx.bin_dir()).map_err(|e| e.to_string()).and_then(|_| {
        ui::info(&format!("downloading whet {}", latest));
        net::fetch(&url, &tmp, false)?;
        toolchain::verify(&tmp, &format!("{}.sha256", url))?;
        toolchain::make_executable(&tmp);
        std::fs::rename(&tmp, ctx.bin_dir().join(Ctx::exe("whet"))).map_err(|e| format!("cannot replace whet: {}", e))
    });
    let _ = std::fs::remove_file(&tmp);
    match r {
        Ok(()) => {
            println!("updated whet {} -> {}", VERSION, latest);
            0
        }
        Err(e) => fail(&e),
    }
}

fn self_uninstall(o: &Opts) -> i32 {
    let (ctx, s) = make_ctx(o);
    if !settings::exists(&ctx.home) {
        return fail(&format!("{} does not look like a whet installation (no settings.toml); refusing to delete it", pretty(&ctx.home)));
    }
    if !o.yes {
        if !ui::can_prompt() {
            return fail("no terminal is available for prompts; re-run with `-y`");
        }
        println!("This will remove {} (all toolchains and whet) and undo the shell profile changes.", pretty(&ctx.home));
        if !ui::ask_yes_no("Continue?", false) {
            println!("Cancelled.");
            return 0;
        }
    }
    let uh = shell::user_home();
    let shell_name = std::env::var("SHELL").unwrap_or_default();
    let mut files: Vec<PathBuf> = s.touched.iter().map(PathBuf::from).collect();
    for f in shell::sh_profiles(&uh, &shell_name) {
        if !files.contains(&f) {
            files.push(f);
        }
    }
    for f in &files {
        match shell::remove_block(f) {
            Ok(true) => {
                ui::info(&format!("removed keen from {}", f.display()));
                // a profile we created (and nobody else wrote to) is left empty: remove it
                if std::fs::read_to_string(f).map_or(false, |t| t.trim().is_empty()) {
                    let _ = std::fs::remove_file(f);
                }
            }
            Ok(false) => {}
            Err(e) => ui::warn(&e),
        }
    }
    for c in &s.created {
        if std::fs::remove_file(c).is_ok() {
            ui::info(&format!("removed {}", c));
        }
    }
    if let Err(e) = std::fs::remove_dir_all(&ctx.home) {
        return fail(&format!("cannot remove `{}`: {}", ctx.home.display(), e));
    }
    println!("keen and whet were uninstalled. Restart your shell to refresh PATH.");
    0
}

fn cmd_completions(o: &Opts) -> i32 {
    let cmds = "init update install default toolchain show which run check self completions help";
    match o.pos.first().map(|s| s.as_str()) {
        Some("bash") => {
            print!(
                "_whet() {{\n    local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n    if [[ $COMP_CWORD -eq 1 ]]; then COMPREPLY=( $(compgen -W \"{c}\" -- \"$cur\") ); return; fi\n    case \"${{COMP_WORDS[1]}}\" in\n        toolchain) COMPREPLY=( $(compgen -W \"list install uninstall\" -- \"$cur\") ) ;;\n        self) COMPREPLY=( $(compgen -W \"update uninstall\" -- \"$cur\") ) ;;\n        completions) COMPREPLY=( $(compgen -W \"bash zsh fish\" -- \"$cur\") ) ;;\n        *) COMPREPLY=( $(compgen -W \"--yes --prefix --dist-server --default-toolchain --profile --no-modify-path --force --check --online\" -- \"$cur\") ) ;;\n    esac\n}}\ncomplete -F _whet whet\n",
                c = cmds
            );
            0
        }
        Some("zsh") => {
            print!("#compdef whet\n_whet() {{\n    local -a cmds\n    cmds=({})\n    if (( CURRENT == 2 )); then compadd -a cmds; return; fi\n    case ${{words[2]}} in\n        toolchain) compadd list install uninstall ;;\n        self) compadd update uninstall ;;\n        completions) compadd bash zsh fish ;;\n    esac\n}}\n_whet \"$@\"\n", cmds);
            0
        }
        Some("fish") => {
            println!("complete -c whet -f");
            for c in cmds.split(' ') {
                println!("complete -c whet -n '__fish_use_subcommand' -a {}", c);
            }
            println!("complete -c whet -n '__fish_seen_subcommand_from toolchain' -a 'list install uninstall'");
            println!("complete -c whet -n '__fish_seen_subcommand_from self' -a 'update uninstall'");
            println!("complete -c whet -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'");
            0
        }
        _ => usage("usage: whet completions <bash|zsh|fish>"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else {
        print!("{}", OVERVIEW);
        exit(0);
    };
    let rest = &args[1..];
    if matches!(cmd.as_str(), "-V" | "--version" | "version") {
        println!("whet {} ({})", VERSION, platform::target());
        exit(0);
    }
    if matches!(cmd.as_str(), "-h" | "--help") || (cmd == "help" && rest.is_empty()) {
        print!("{}", OVERVIEW);
        exit(0);
    }
    if cmd == "help" {
        print!("{}", if rest[0] == "init" { INIT_HELP } else { OVERVIEW });
        exit(0);
    }
    if cmd == "run" {
        // everything after the version belongs to the command being run, so no whet option parsing
        ui::setup(false, false);
        exit(cmd_run(&Opts::default(), rest));
    }
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", if cmd == "init" { INIT_HELP } else { OVERVIEW });
        exit(0);
    }
    let o = match parse_opts(rest) {
        Ok(o) => o,
        Err(e) => exit(usage(&e)),
    };
    ui::setup(o.quiet, o.verbose);
    let code = match cmd.as_str() {
        "init" => cmd_init(&o),
        "update" => cmd_update(&o),
        "install" => cmd_install(&o, o.pos.first()),
        "default" => cmd_default(&o, o.pos.first()),
        "toolchain" => cmd_toolchain(&o),
        "show" => cmd_show(&o),
        "which" => cmd_which(&o),
        "check" | "doctor" => cmd_check(&o),
        "self" => cmd_self(&o),
        "completions" => cmd_completions(&o),
        other => usage(&format!("unknown command `{}`", other)),
    };
    exit(code);
}
