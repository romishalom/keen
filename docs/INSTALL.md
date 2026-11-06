# Installing keen with whet

**whet** (as in *whetstone*: it sharpens keen) installs and manages keen toolchains, the way rustup
does for Rust. keen itself needs a C compiler (gcc or clang) to build programs; whet warns you if
there is none and tells you how to get one.

## The easy way

```sh
curl --proto '=https' --tlsv1.2 -sSf https://keen-lang.org/install.sh | sh
```

You get a short menu; press **Enter** to accept the standard installation:

```
Welcome to keen!

Current installation options:

   install dir:         ~/.keen
   default toolchain:   latest (resolved at install time)
   profile:             default
   modify PATH:         yes
   dist server:         https://keen-lang.org/dist

1) Proceed with standard installation (default - just press enter)
2) Customize installation
3) Cancel installation
>
```

Then open a new shell (or run `. "$HOME/.keen/env"`) and try:

```sh
keen new hello && cd hello && keen run
```

No prompts, for scripts and CI: `curl ... | sh -s -- -y`.

Windows (PowerShell, not yet tested on a real machine):
`irm https://keen-lang.org/install.ps1 | iex`

> The host name above is a placeholder. Releases are plain static files, so any web server works;
> see [Publishing releases](#publishing-releases) to point the scripts at yours.

## The advanced way

Everything after `sh -s --` goes to `whet init`:

| option | meaning |
|---|---|
| `-y`, `--yes` | no prompts; use the defaults plus the options below |
| `--default-toolchain <v>` | `latest` (default), a version like `0.2.1`, or `none` (install only whet) |
| `--profile <p>` | `default` (keen + shell completions) or `minimal` (keen only) |
| `--no-modify-path` | do not edit shell profiles (an `env` file is still written to source by hand) |
| `--prefix <dir>` | install root instead of `~/.keen` (also `$KEEN_HOME`) |
| `--dist-server <url>` | install from a mirror: `https://`, `file://` (offline) or loopback `http://` |
| `--target <triple>` | override the detected platform |
| `--force` | reinstall even if the toolchain is already present |
| `--no-cc-check` | do not warn about a missing C compiler |
| `-q`, `-v` | quieter / more verbose |

```sh
# pinned, minimal, system-wide-ish, no shell edits
curl --proto '=https' --tlsv1.2 -sSf https://keen-lang.org/install.sh | \
    sh -s -- -y --default-toolchain 0.2.1 --profile minimal --no-modify-path --prefix /opt/keen

# fully offline, from a directory of release files
sh install.sh -y --dist-server file:///mnt/usb/keen-dist
```

Bootstrap environment variables: `WHET_DIST_SERVER` (release server), `WHET_VERSION` (which whet to
bootstrap), `WHET_TARGET` (platform override), `WHET_ALLOW_HTTP=1` (permit plain http to a remote
host, off by default), `NO_COLOR`.

## What whet changes on your machine

```
~/.keen/                    everything lives here (override with --prefix / $KEEN_HOME)
    bin/keen                symlink to the default toolchain
    bin/whet                the manager
    toolchains/<version>/   one directory per installed version
    share/completions/      keen's bash/zsh completions (default profile)
    env                     sourced by your shell profile to put bin/ on PATH
    settings.toml           what whet did, so it can undo it
```

Outside `~/.keen`, whet only touches your **shell profiles**, and only to add this block (once;
re-running changes nothing):

```sh
# >>> keen (whet) >>>
. "$HOME/.keen/env"
# <<< keen (whet) <<<
```

It edits `~/.profile` (created if needed), plus `~/.bashrc`, `~/.bash_profile` and `~/.zshenv` when
they exist or match `$SHELL`. fish users get `~/.config/fish/conf.d/keen.fish` and completions. The
list of touched files is recorded in `settings.toml`; `whet self uninstall` reverses every change.

Downloads are always checked against a SHA-256 published next to them, archives with unsafe paths
are refused, and plain `http://` to a remote host is rejected.

## Managing keen

| command | |
|---|---|
| `whet update [--check]` | install the newest keen and make it the default (never downgrades) |
| `whet install <v>` / `whet toolchain install <v>` | install another version (`latest` or `0.2.1`) |
| `whet default <v>` | switch the active version (instant rollback) |
| `whet toolchain list` / `uninstall <v>` | see / remove versions |
| `whet run <v> <cmd> ...` | run a command with a specific toolchain first on PATH |
| `whet show`, `whet which` | inspect the installation |
| `whet check [--online]` | diagnose PATH, links, the C compiler and the release server |
| `whet self update` | update whet itself |
| `whet self uninstall [-y]` | remove `~/.keen` and undo the profile changes |
| `whet completions <shell>` | completion script for whet |

## Troubleshooting

- **`keen: command not found`**: open a new shell, or run `. "$HOME/.keen/env"`. `whet check` tells
  you exactly what is missing.
- **no C compiler**: Debian/Ubuntu `sudo apt install build-essential`, Fedora `sudo dnf install gcc`,
  macOS `xcode-select --install`. `keen doctor` shows what keen found.
- **`checksum mismatch`**: the download was corrupted or altered; nothing was installed. Retry, or
  check your mirror.
- **another `keen` earlier on PATH** (for example from `cargo install`): whet warns about it; put
  `~/.keen/bin` first.
- **behind a proxy**: whet uses `curl`/`wget`, so `https_proxy` etc. are honoured.

## Publishing releases

`scripts/package-release.sh` builds a release and writes the layout `whet` and `install.sh` expect:

```
<out>/latest                                 newest version, one line
<out>/install.sh  <out>/install.ps1          the bootstrap scripts
<out>/keen/<ver>/keen-<ver>-<target>.tar.gz  toolchain archive (+ .sha256)
<out>/whet/<ver>/whet-<target>[.exe]         manager binary (+ .sha256)
```

```sh
WHET_DEFAULT_DIST_SERVER=https://downloads.example.org/keen \
    sh scripts/package-release.sh --out dist            # builds with your server baked in
# repeat on each platform (or with --target), then upload dist/ to the server
```

Set the same URL as the default in `scripts/install.sh` / `install.ps1` (the `DIST=` line) or have
users export `WHET_DIST_SERVER`. Targets look like `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin`, `x86_64-unknown-linux-musl`.
