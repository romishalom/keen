//! Shell completion scripts (`keen completions bash|zsh|fish`).

use keen::explain::ENTRIES;

pub const COMMANDS: &[(&str, &str)] = &[
    ("new", "Create a new project in a new directory"),
    ("init", "Create a project in the current directory"),
    ("build", "Compile a project or file to an executable"),
    ("run", "Build and run"),
    ("test", "Build and run #[test] functions"),
    ("check", "Transpile and syntax-check with the C compiler"),
    ("fmt", "Format keen source files"),
    ("emit", "Transpile to C only"),
    ("clean", "Remove build artifacts"),
    ("doctor", "Show the detected C compiler and environment"),
    ("explain", "Explain a diagnostic code"),
    ("completions", "Print a shell completion script"),
    ("help", "Show help"),
    ("version", "Show the version"),
];

pub const FLAGS: &[(&str, &str)] = &[
    ("--release", "Optimize (-O2), no debug info"),
    ("--debug", "Always pass -g"),
    ("--cc", "C compiler to use"),
    ("-o", "Output path"),
    ("--out-dir", "Directory for generated C"),
    ("--keep", "Keep the temporary build directory"),
    ("--verbose", "Print every step"),
    ("--quiet", "Print less"),
    ("--no-warnings", "Silence C compiler warnings"),
    ("--watch", "Re-run when files change"),
    ("--color", "auto, always or never"),
    ("--std=c11", "Target C11"),
    ("--std=c23", "Target C23"),
    ("--no-line-directives", "Do not emit #line directives"),
    ("--minimal-parens", "Parenthesize only where C requires it"),
    ("--mangle-sep=", "Separator for mangled names"),
    ("--no-case-ranges", "Reject range patterns in match"),
    ("--split", "Write a .h/.c pair"),
    ("--module", "Compile as module a::b"),
    ("--check", "fmt: exit 1 if files would change"),
    ("--stdin", "fmt: read stdin, write stdout"),
    ("--stdout", "fmt: print instead of writing"),
    ("--help", "Show help"),
];

fn names() -> String {
    COMMANDS.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(" ")
}

fn codes() -> String {
    ENTRIES.iter().map(|e| e.code).collect::<Vec<_>>().join(" ")
}

pub fn script(shell: &str) -> Option<String> {
    match shell {
        "bash" => Some(bash()),
        "zsh" => Some(zsh()),
        "fish" => Some(fish()),
        _ => None,
    }
}

fn bash() -> String {
    let opts = FLAGS.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(" ");
    format!(
        r#"# bash completion for keen: source <(keen completions bash)
_keen() {{
    local cur prev
    COMPREPLY=()
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev="${{COMP_WORDS[COMP_CWORD-1]}}"
    local cmds="{cmds}"
    local opts="{opts} -O0 -O1 -O2 -O3 -Os -g -w -v -q -D -I -L -l -C"
    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "$cmds" -- "$cur") )
        return
    fi
    case "${{COMP_WORDS[1]}}" in
        explain) COMPREPLY=( $(compgen -W "{codes}" -- "$cur") ); return ;;
        completions) COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") ); return ;;
        help) COMPREPLY=( $(compgen -W "$cmds" -- "$cur") ); return ;;
    esac
    case "$prev" in
        --cc) COMPREPLY=( $(compgen -c -- "$cur") ); return ;;
        --color) COMPREPLY=( $(compgen -W "auto always never" -- "$cur") ); return ;;
        -o|--out-dir) COMPREPLY=( $(compgen -f -- "$cur") ); return ;;
    esac
    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
    else
        COMPREPLY=( $(compgen -f -X '!*.kn' -- "$cur") $(compgen -d -- "$cur") )
    fi
}}
complete -o filenames -F _keen keen
"#,
        cmds = names(),
        opts = opts,
        codes = codes()
    )
}

fn zsh() -> String {
    let cmds = COMMANDS.iter().map(|(c, d)| format!("        '{}:{}'", c, d)).collect::<Vec<_>>().join("\n");
    let opts = FLAGS.iter().map(|(f, _)| format!("'{}'", f)).collect::<Vec<_>>().join(" ");
    format!(
        r#"#compdef keen
# zsh completion for keen: keen completions zsh > ~/.zfunc/_keen
_keen() {{
    local -a commands opts
    commands=(
{cmds}
    )
    opts=({opts} '-O0' '-O1' '-O2' '-O3' '-Os' '-g' '-w' '-v' '-q')
    if (( CURRENT == 2 )); then
        _describe -t commands 'keen command' commands
        return
    fi
    case ${{words[2]}} in
        explain) compadd {codes}; return ;;
        completions) compadd bash zsh fish; return ;;
        help) _describe -t commands 'keen command' commands; return ;;
    esac
    if [[ $PREFIX == -* ]]; then
        compadd -a opts
    else
        _files
    fi
}}
_keen "$@"
"#,
        cmds = cmds,
        opts = opts,
        codes = codes()
    )
}

fn fish() -> String {
    let mut s = String::from("# fish completion for keen: keen completions fish > ~/.config/fish/completions/keen.fish\ncomplete -c keen -f\n");
    for (c, d) in COMMANDS {
        s.push_str(&format!("complete -c keen -n '__fish_use_subcommand' -a {} -d '{}'\n", c, d));
    }
    for (f, d) in FLAGS {
        if let Some(long) = f.strip_prefix("--") {
            let long = long.trim_end_matches('=').split('=').next().unwrap_or(long);
            s.push_str(&format!("complete -c keen -l {} -d '{}'\n", long, d));
        } else if let Some(short) = f.strip_prefix('-') {
            s.push_str(&format!("complete -c keen -s {} -d '{}'\n", short, d));
        }
    }
    s.push_str(&format!("complete -c keen -n '__fish_seen_subcommand_from explain' -a '{}'\n", codes()));
    s.push_str("complete -c keen -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'\n");
    s.push_str(&format!("complete -c keen -n '__fish_seen_subcommand_from help' -a '{}'\n", names()));
    s.push_str("complete -c keen -n '__fish_seen_subcommand_from build run test check fmt emit' -a '(__fish_complete_suffix .kn)'\n");
    s
}
