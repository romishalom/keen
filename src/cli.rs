//! Command-line parsing and help texts for the `keen` binary.

use keen::Options;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    New,
    Init,
    Build,
    Run,
    Test,
    Check,
    Fmt,
    Emit,
    Clean,
    Doctor,
    Explain,
    Completions,
    Help,
    Version,
}

impl Cmd {
    pub fn name(self) -> &'static str {
        match self {
            Cmd::New => "new",
            Cmd::Init => "init",
            Cmd::Build => "build",
            Cmd::Run => "run",
            Cmd::Test => "test",
            Cmd::Check => "check",
            Cmd::Fmt => "fmt",
            Cmd::Emit => "emit",
            Cmd::Clean => "clean",
            Cmd::Doctor => "doctor",
            Cmd::Explain => "explain",
            Cmd::Completions => "completions",
            Cmd::Help => "help",
            Cmd::Version => "version",
        }
    }

    fn from_name(s: &str) -> Option<Cmd> {
        Some(match s {
            "new" => Cmd::New,
            "init" => Cmd::Init,
            "build" | "b" => Cmd::Build,
            "run" | "r" => Cmd::Run,
            "test" | "t" => Cmd::Test,
            "check" | "c" => Cmd::Check,
            "fmt" | "format" => Cmd::Fmt,
            "emit" | "transpile" => Cmd::Emit,
            "clean" => Cmd::Clean,
            "doctor" => Cmd::Doctor,
            "explain" => Cmd::Explain,
            "completions" => Cmd::Completions,
            "help" => Cmd::Help,
            "version" => Cmd::Version,
            _ => return None,
        })
    }
}

#[derive(Debug)]
pub struct Cli {
    pub cmd: Cmd,
    /// Positional arguments (files, directories, names, codes ...).
    pub inputs: Vec<String>,
    pub output: Option<String>,
    /// Arguments after `--` (for `run` and `test`).
    pub prog_args: Vec<String>,
    pub cc: Option<String>,
    pub opt: Option<String>,
    pub release: bool,
    pub debug: bool,
    pub warnings: bool,
    pub cflags: Vec<String>,
    pub ldflags: Vec<String>,
    pub out_dir: Option<String>,
    pub keep: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub watch: bool,
    pub color: String,
    /// `Some(true)` for an explicit `--std=c23`, `Some(false)` for `--std=c11`.
    pub std_explicit: Option<bool>,
    // fmt
    pub fmt_check: bool,
    pub fmt_stdin: bool,
    pub fmt_stdout: bool,
    pub topts: Options,
}

fn blank(cmd: Cmd) -> Cli {
    Cli {
        cmd,
        inputs: Vec::new(),
        output: None,
        prog_args: Vec::new(),
        cc: None,
        opt: None,
        release: false,
        debug: false,
        warnings: true,
        cflags: Vec::new(),
        ldflags: Vec::new(),
        out_dir: None,
        keep: false,
        verbose: false,
        quiet: false,
        watch: false,
        color: "auto".to_string(),
        std_explicit: None,
        fmt_check: false,
        fmt_stdin: false,
        fmt_stdout: false,
        topts: Options::default(),
    }
}

fn value(it: &mut std::vec::IntoIter<String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("`{}` needs a value", flag))
}

pub fn parse(args: Vec<String>) -> Result<Cli, String> {
    let mut rest = args;
    // `keen`, `keen --help`, `keen --version`
    let first = rest.first().cloned();
    let cmd = match first.as_deref() {
        None | Some("-h") | Some("--help") => {
            return Ok(blank(Cmd::Help));
        }
        Some("-V") | Some("--version") => return Ok(blank(Cmd::Version)),
        Some(s) => match Cmd::from_name(s) {
            Some(c) => {
                rest.remove(0);
                c
            }
            // Backward compatible: `keen file.kn [options]` just transpiles.
            None => Cmd::Emit,
        },
    };

    let mut cli = blank(cmd);
    let mut it = rest.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => cli.prog_args = it.by_ref().collect(),
            "-h" | "--help" => {
                // `keen build --help` shows help for build
                let topic = cli.cmd.name().to_string();
                let mut h = blank(Cmd::Help);
                h.inputs.push(topic);
                return Ok(h);
            }
            "-V" | "--version" => return Ok(blank(Cmd::Version)),
            "-o" => cli.output = Some(value(&mut it, "-o")?),
            "--out-dir" => cli.out_dir = Some(value(&mut it, "--out-dir")?),
            "--cc" => cli.cc = Some(value(&mut it, "--cc")?),
            "--color" => {
                let v = value(&mut it, "--color")?;
                if !matches!(v.as_str(), "auto" | "always" | "never") {
                    return Err("`--color` takes auto, always or never".to_string());
                }
                cli.color = v;
            }
            "--release" => cli.release = true,
            "--debug" | "-g" => cli.debug = true,
            "--no-warnings" | "-w" => cli.warnings = false,
            "--keep" => cli.keep = true,
            "--watch" => cli.watch = true,
            "-v" | "--verbose" => cli.verbose = true,
            "-q" | "--quiet" => cli.quiet = true,
            "-C" | "--cflag" => cli.cflags.push(value(&mut it, &a)?),
            "--ldflag" => cli.ldflags.push(value(&mut it, "--ldflag")?),
            "-D" | "-I" => {
                let v = value(&mut it, &a)?;
                cli.cflags.push(format!("{}{}", a, v));
            }
            "-L" | "-l" => {
                let v = value(&mut it, &a)?;
                cli.ldflags.push(format!("{}{}", a, v));
            }
            s if s.len() > 2 && (s.starts_with("-D") || s.starts_with("-I")) => cli.cflags.push(a.clone()),
            s if s.len() > 2 && (s.starts_with("-L") || s.starts_with("-l")) => cli.ldflags.push(a.clone()),
            s if s.starts_with("-O") => {
                let lvl = &s[2..];
                cli.opt = Some(if lvl.is_empty() { "1".to_string() } else { lvl.to_string() });
            }
            // fmt
            "--check" => cli.fmt_check = true,
            "--stdin" => cli.fmt_stdin = true,
            "--stdout" => cli.fmt_stdout = true,
            // transpiler options
            "--split" => cli.topts.split = true,
            "--no-line-directives" => cli.topts.line_directives = false,
            "--minimal-parens" => cli.topts.minimal_parens = true,
            "--no-case-ranges" => cli.topts.case_ranges = false,
            "--module" => cli.topts.module = Some(value(&mut it, "--module")?),
            "--std=c11" => {
                cli.topts.std23 = false;
                cli.std_explicit = Some(false);
            }
            "--std=c23" | "--std=c2x" => {
                cli.topts.std23 = true;
                cli.std_explicit = Some(true);
            }
            s if s.starts_with("--mangle-sep=") => cli.topts.sep = s["--mangle-sep=".len()..].to_string(),
            "-" => cli.inputs.push(a),
            s if s.starts_with('-') => {
                return Err(format!("unknown option `{}` (try `keen help {}`)", s, cli.cmd.name()));
            }
            _ => cli.inputs.push(a),
        }
    }
    if !cli.prog_args.is_empty() && !matches!(cli.cmd, Cmd::Run | Cmd::Test) {
        return Err("arguments after `--` are only used by `keen run` and `keen test`".to_string());
    }
    Ok(cli)
}

// ------------------------------------------------------------------------- help

const OVERVIEW: &str = "\
keen - Rust-syntax to C: transpiler, build tool, test runner and formatter
(syntax only: no borrow checker, no type checker, no safety enforcement)

USAGE:
    keen <command> [options] [files]
    keen <file.kn> [options]              same as `keen emit <file.kn>`

PROJECT COMMANDS:
    new <name>     Create a new project in ./<name>
    init           Create a project in the current directory
    build          Compile to an executable          (target/debug/<name>, or ./<file>)
    run            Build and run                     (program arguments after `--`)
    test           Build and run #[test] functions    (`keen test <filter>`, `-- --list`)
    check          Transpile + C syntax check, no binary
    fmt            Format source files               (--check for CI, --stdin for editors)
    clean          Remove build artifacts

OTHER COMMANDS:
    emit           Transpile one file to C and stop
    doctor         Show the detected C compiler and environment
    explain <E…>   Explain a diagnostic code (`keen explain` lists them)
    completions    Print a shell completion script (bash, zsh, fish)
    help [cmd]     Show this help, or help for a command
    version        Show the version

Run `keen help <command>` for options. A project is a directory with a keen.toml; commands run
without a file find it by walking up from the current directory.
";

const CC_OPTS: &str = "\
C COMPILER OPTIONS:
    --cc <prog>          Compiler to use; may include flags (\"clang\", \"gcc -m32\").
                         Default: $KEEN_CC, then $CC, then cc, gcc, clang.
    --release            -O2 and no -g (default is -O0 -g); project output goes to target/release
    -O0|-O1|-O2|-O3|-Os  Optimization level            -g, --debug   Always pass -g
    -w, --no-warnings    Silence C compiler warnings (default: -Wall -Wextra)
    -D NAME[=V]          Define a macro (use with #[cfg(NAME)])      -I <dir>   Include directory
    -l <lib>, -L <dir>   Link a library / add a library directory
    -C <flag>            Pass any other flag to the C compiler       --ldflag <flag>
    --out-dir <dir>      Where generated C goes
    -v, --verbose        Print every step and command                -q, --quiet   Less output
    --watch              Re-run whenever a .kn/.c/.h/keen.toml file changes
    --color <when>       auto, always or never

TRANSPILER OPTIONS:
    --std=c11|c23        Target C standard (default c11, or `std` in keen.toml)
    --no-line-directives Do not emit #line directives (C errors then point at generated C)
    --minimal-parens     Parenthesize only where C requires it
    --mangle-sep=<sep>   Separator for mangled names (default `_`)
    --no-case-ranges     Reject range patterns in `match`
";

pub fn help(topic: Option<&str>) -> String {
    let Some(t) = topic else { return OVERVIEW.to_string() };
    let cmd = Cmd::from_name(t);
    let body = match cmd {
        Some(Cmd::New) => "keen new <name>\n\nCreate ./<name> with keen.toml, src/main.kn (hello world + a test) and .gitignore.\n".to_string(),
        Some(Cmd::Init) => "keen init [name]\n\nCreate keen.toml (and src/main.kn if missing) in the current directory. The package\nname defaults to the directory name.\n".to_string(),
        Some(Cmd::Build) => format!("keen build [options] [file.kn] [extra .c/.o/.a files]\n\nTranspile and compile to an executable. In a project: target/<profile>/<name> (rebuilds only\nwhen something changed). With a file: ./<file stem> (or -o <path>), generated C in .keen/<stem>.\nFile modules (`mod name;`) are found next to the entry file and compiled too.\n\n    -o <path>            Executable path (file mode, or to override the project output)\n\n{}", CC_OPTS),
        Some(Cmd::Run) => format!("keen run [options] [file.kn] [extra files] [-- program args]\n\nBuild and run, forwarding the program's exit code. Without a project, the build happens in a\ntemporary directory (use --keep to retain it).\n\n    --keep               Keep the temporary build directory\n\n{}", CC_OPTS),
        Some(Cmd::Test) => format!("keen test [options] [file.kn] [filter...] [-- --list | --nocapture]\n\nBuild with `main` replaced by a test runner and run every #[test] function (in the entry file\nand its file modules). A test passes if it returns normally (or returns 0); a failed assert, a\ncrash or a non-zero return fails it. Each test runs in its own process with captured output.\nFilters are substrings of the test names (`util::clamp_works`).\n\n{}", CC_OPTS),
        Some(Cmd::Check) => format!("keen check [options] [file.kn]\n\nTranspile and run the C compiler with -fsyntax-only. Diagnostics point at your .kn files.\n\n{}", CC_OPTS),
        Some(Cmd::Fmt) => "keen fmt [options] [paths...]\n\nFormat .kn files in place (directories are searched recursively; with no paths, the project or\nthe current directory). The formatter keeps your line breaks and normalizes indentation,\nspacing, braces, blank lines and trailing whitespace; comments are preserved.\n\n    --check              Do not write; print a diff and exit 1 if any file would change\n    --stdin              Read source from stdin, write the formatted text to stdout\n    --stdout             Print the formatted text instead of writing the file\n    -q, --quiet          Do not list formatted files\n".to_string(),
        Some(Cmd::Emit) => "keen emit [options] <file.kn>\n\nTranspile a single file to C (no compiler needed).\n\n    -o <out.c>           Output file (default: input with .c); `-o -` writes to stdout\n    --split              Write <stem>.h (all `pub` items) and <stem>.c\n    --module <a::b>      Compile as module a::b (prefixes top-level names)\n    --std=c11|c23   --no-line-directives   --minimal-parens   --mangle-sep=<s>   --no-case-ranges\n".to_string(),
        Some(Cmd::Clean) => "keen clean [--out-dir <dir>]\n\nRemove target/ and .keen/ (the project's, or the current directory's).\n".to_string(),
        Some(Cmd::Doctor) => "keen doctor [--cc <prog>]\n\nShow the C compiler keen would use and whether it supports the C standards keen targets.\n".to_string(),
        Some(Cmd::Explain) => "keen explain [code]\n\nPrint a long description of a diagnostic, e.g. `keen explain E0201`. Without a code, list all.\n".to_string(),
        Some(Cmd::Completions) => "keen completions <bash|zsh|fish>\n\nPrint a completion script, e.g.:\n    source <(keen completions bash)\n    keen completions zsh > ~/.zfunc/_keen\n    keen completions fish > ~/.config/fish/completions/keen.fish\n".to_string(),
        Some(Cmd::Help) | Some(Cmd::Version) => OVERVIEW.to_string(),
        None => format!("keen: no help for `{}`\n\n{}", t, OVERVIEW),
    };
    body
}
