# keen

A Rust-syntax front end for C: **transpiler, build tool, test runner and formatter** in one
dependency-free binary. keen changes C's *syntax* only; all of C's semantics (integer promotion,
undefined behavior, ABI, linkage) are preserved, and there is **no borrow checker, no type checker
and no safety enforcement**. The C compiler (gcc or clang) does the semantic checking, and keen maps
its diagnostics back to your `.kn` source with exact `#line` directives.

```rust
use <stdio.h>;

struct Point { x: f32, y: f32 }

impl Point {
    pub fn len2(p: *const Point) -> f32 { p->x * p->x + p->y * p->y }
}

pub fn main() -> i32 {
    let p = Point { x: 3.0, y: 4.0 };
    printf("%.1f\n", Point::len2(&p) as f64);
    return 0;
}
```

```
$ keen run point.kn
25.0
```

## Install

```
cargo build --release          # binary: target/release/keen   (needs gcc or clang to build/run .kn)
cargo test                     # 93 tests; builds and runs real programs with gcc (and clang if present)
```

Shell completions: `source <(keen completions bash)`, `keen completions zsh > ~/.zfunc/_keen`,
`keen completions fish > ~/.config/fish/completions/keen.fish`.

## Quick start

```
keen new hello && cd hello
keen run          # build + run                      (target/debug/hello)
keen test         # run #[test] functions
keen fmt          # format the project
keen build --release
```

## Commands

| command | what it does |
|---|---|
| `keen new <name>` / `keen init` | scaffold a project (`keen.toml`, `src/main.kn`, `.gitignore`) |
| `keen build` | transpile + compile; incremental (skips work when nothing changed) |
| `keen run [-- args]` | build and run, forwarding the program's exit code |
| `keen test [filter] [-- --list \| --nocapture]` | build with a test runner and run every `#[test]` fn |
| `keen check` | transpile + `cc -fsyntax-only`, no binary |
| `keen fmt [paths]` | format files in place; `--check` (CI), `--stdin`, `--stdout` |
| `keen emit file.kn` | transpile one file to C and stop (`--split` for `.h`/`.c`) |
| `keen clean` | remove `target/` and `.keen/` |
| `keen doctor` | show the C compiler, supported C standards, project info |
| `keen explain E0201` | long explanation of a diagnostic code (no code: list them) |
| `keen completions <shell>` | bash / zsh / fish completion scripts |
| `keen help [command]` | overall or per-command help (`keen build --help` works too) |

Every build-ish command works on a **project** (a directory with `keen.toml`, found by walking up
from the current directory) or on a single file: `keen run file.kn`, `keen build file.kn -o app`,
`keen test file.kn`. File modules (`mod util;`, `util.kn` or `util/mod.kn` next to the declaring
file, nested modules in `util/text.kn`) are discovered and compiled automatically.

Useful flags: `--release`, `-O0..-O3/-Os`, `-g`, `--cc clang` (also `$KEEN_CC`, `$CC`), `-D NAME`
(with `#[cfg(NAME)]`), `-I dir`, `-l lib`, `-L dir`, `-C <cc flag>`, extra `.c/.o/.a` files as
positional arguments, `--std=c11|c23`, `-w`, `-v`, `-q`, `--watch`, `--color auto|always|never`.

### keen.toml

```toml
[package]
name = "hello"
version = "0.1.0"
entry = "src/main.kn"      # optional

[build]
std = "c11"                # or "c23"
cc = "clang"               # optional
opt = "2"                  # optional
warnings = true
cflags = ["-DFEATURE_X"]
ldflags = ["-lm"]
sources = ["c/helper.c"]   # extra C files compiled and linked in
```

## Testing

```rust
use <assert.h>;

fn add(a: i32, b: i32) -> i32 { a + b }

#[test]
fn adds() { assert(add(1, 2) == 3); }
```

`keen test` replaces your `main` with a runner. A test passes if it returns normally (or returns
0) and fails on a non-zero return, a failed `assert`, or a crash. Each test runs in its own process
with captured output, so one failure never stops the rest. Tests in file modules are reported as
`util::name`. `#[test]` functions are not compiled into normal builds.

## Formatting

`keen fmt` is a comment-preserving, layout-preserving formatter: it keeps your line breaks and
normalizes indentation (4 spaces), spacing, brace placement (`} else {`), one statement per line,
blank lines and trailing whitespace. Its output is always re-lexed and compared with the input, so
it can never change what a program means, and formatting is idempotent. A file's line-ending style is
preserved (CRLF stays CRLF, LF stays LF), so `keen fmt --check` is quiet on Windows checkouts.

## Line endings

CRLF and LF sources are equivalent: the lexer normalizes line endings first, so raw strings, `c!`
blocks and `#line` numbers mean the same thing either way. Generated C always uses LF.

## Diagnostics

```
error[E0201]: comparison operators cannot be chained; use `&&`
 --> demo.kn:3:14
  |
3 |     if a < 1 < 2 { return 1; }
  |              ^
  = help: run `keen explain E0201` for details
```

C compiler errors and warnings point at `.kn` files and lines. Exit codes: `0` success, `1` a
build/test/format failure (or the program's own exit code for `run`), `2` usage error.

## Language at a glance

| keen | C |
|---|---|
| `fn add(a: i32, b: i32) -> i32 { a + b }` | `static int32_t add(int32_t a, int32_t b) { return a + b; }` |
| `pub fn` | external linkage (no `pub` means `static`) |
| `let x: i32 = 1;` / `let mut x = f();` | `const int32_t x = 1;` / `__auto_type x = f();` |
| `*mut T`, `*const T`, `[T; N]`, `fn(i32) -> i32` | `T*`, `const T*`, `T[N]`, `int32_t (*)(int32_t)` |
| `x as u8` | `(uint8_t)x` |
| `if c { a } else { b }` (expression) | `c ? a : b` |
| `match x { 1 \| 2 => .., 3..=5 => .., _ => .. }` | `switch` with automatic `break` |
| `for i in 0..n`, `for let mut i = 0; i < n; i += 2 { }` | C `for` loops |
| `'outer: loop { break 'outer; }` | `goto`-based labeled break/continue |
| `struct S { a: u32 : 3 }`, `union`, `enum`, `type` | bit-fields, `union`, prefixed `enum`, `typedef` |
| `impl S { fn f() }` / `S::f()`, `mod m { fn g() }` / `m::g()` | `S_f`, `m_g` |
| `use <stdio.h>;` | `#include <stdio.h>` |
| `#[cfg(DEBUG)]`, `#[packed]`, `#[align(16)]`, `#[inline]`, `#[test]` | `#ifdef`, `__attribute__`, `_Alignas`, `inline`, test runner |
| `c!("#define X 1");` | raw C, verbatim |

The full language specification is [`docs/SPEC.md`](docs/SPEC.md) (with an appendix of where the
implementation clarifies it, and one on the tooling). Runnable samples are in `examples/`
(`examples/project/` is a multi-file project).

Things to know: `!` is logical not and `~` is bitwise not; assignment is a statement (no `++`/`--`
or `?:`); `.` is for values and `->` for pointers; bitwise operators bind tighter than comparisons
as in Rust (the emitted C is parenthesized to match); types and functions can be declared in any
order.

## Layout

```
src/lexer.rs       tokens, comments (formatter), byte spans
src/parser.rs      recursive-descent parser -> AST          src/ast.rs   syntax tree (no types)
src/emit.rs        mangling, declarators, ordering, statements/expressions, #[test] registration
src/harness.rs     the C test runner used by `keen test`
src/fmt.rs         the formatter (+ diff for --check)
src/driver.rs      C compiler detection, module discovery, build/check/run, incremental builds
src/manifest.rs    keen.toml (tiny TOML subset)             src/explain.rs   `keen explain` texts
src/cli.rs         argument parsing and help                src/completions.rs   shell completions
src/main.rs        command dispatch, projects, fmt, test, watch
tests/             transpiler, formatter, CLI, project and example tests
```
