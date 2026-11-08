# keen v0.3.0 — Initial Release

We are excited to announce the initial release of **keen** (`v0.3.0`), a Rust-syntax front end for C: transpiler, build tool, test runner, and formatter packed into a single dependency-free binary.

Along with the language toolchain, this release introduces **whet**, our dedicated toolchain manager designed to install, update, and manage your `keen` versions.

## Key Features

* **Rust-Like Syntax for C**: Expressive control flow, modern type annotation syntax (`let`, `fn`, `impl`, `match`, `*mut T`, `*const T`), and pattern matching while retaining full C semantics (no borrow checker, no ABI changes, and no added overhead).

* **Exact C Diagnostics Mapping**: `keen` maps diagnostics directly back to your original `.kn` source files via `#line` directives.

* **Integrated Test Runner**: Built-in `keen test` harness with isolated per-test processes, output capturing, and automatic module reporting.

* **Idempotent Formatter**: `keen fmt` normalizes code layout while guaranteeing zero semantic changes by re-lexing output against source ASTs.

* **Zero-Configuration Build System**: Simple project configuration (`keen.toml`) supporting incremental builds, module discovery, single-file compilation, and C target integration.

## Included Tools

### 1. `keen` CLI

The primary command-line tool for project initialization, compilation, testing, and formatting:

| Command | Description | 
| ----- | ----- | 
| `keen new <name>` / `keen init` | Scaffold a new project (`keen.toml`, `src/main.kn`, `.gitignore`) | 
| `keen build` | Incremental transpilation and C compilation | 
| `keen run [-- args]` | Build and execute your application directly | 
| `keen test [filter]` | Run all `#[test]` functions in isolated processes | 
| `keen check` | Transpile and run C syntax checks without building a binary | 
| `keen fmt [paths]` | Format source files in-place | 
| `keen emit file.kn` | Transpile a `.kn` source file to standard C code | 
| `keen doctor` | Display C compiler detection, supported standards, and environment info | 

### 2. `whet` Toolchain Manager

`whet` manages `keen` versions much like `rustup` manages Rust:

* **Toolchain Switching**: Install and toggle between multiple toolchain versions (`whet install`, `whet default`).

* **Environment Verification**: Diagnostic checks via `whet check` to inspect PATHs, links, and system C compiler availability.

* **Seamless Updates**: Update the manager or language binaries using `whet update` and `whet self update`.

## Quick Start & Installation

### Automatic Install (macOS / Linux)

Run the bootstrap script to install `whet` and the `v0.3.0` release into `~/.keen`:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://keen-lang.org/install.sh | sh
```

For unattended / CI environments:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://keen-lang.org/install.sh | sh -s -- -y --default-toolchain 0.3.0 --profile minimal
```

### Windows (PowerShell)

```powershell
irm https://keen-lang.org/install.ps1 | iex
```

### Building From Source

Requirements: A standard Rust toolchain and a C compiler (`gcc` or `clang`).

```sh
cargo build --release
```

Binaries will be placed in `target/release/keen` and `target/release/whet`.

## Example

```rust
use <stdio.h>;

struct Point { x: f32, y: f32 }

impl Point {
    pub fn len2(p: *const Point) -> f32 { 
        p->x * p->x + p->y * p->y 
    }
}

pub fn main() -> i32 {
    let p = Point { x: 3.0, y: 4.0 };
    printf("%.1f\n", Point::len2(&p) as f64);
    return 0;
}
```

Run it directly with:

```sh
keen run point.kn
```

## Feedback & Contributions

Since this is the inaugural release, we welcome your feedback, bug reports, and suggestions! Please open an issue on our repository or check out our documentation files in `docs/` for details on language specifications and toolchain deployment.