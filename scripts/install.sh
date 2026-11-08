#!/bin/sh
# whet: the keen installer (bootstrap script)
#
#   curl --proto '=https' --tlsv1.2 -sSf https://github.com/romishalom/keen/releases/latest/download/install.sh | sh
#
# Pass options to the installer after `-s --`:
#
#   curl ... | sh -s -- -y                                  # no prompts, all defaults
#   curl ... | sh -s -- --default-toolchain 0.3.0 --profile minimal --no-modify-path
#
# This script only downloads `whet` (verifying its SHA-256) and runs `whet init`, which does the
# real work. Run `whet help init` afterwards for every option.
#
# Environment: WHET_DIST_SERVER (release server), WHET_VERSION (which whet to bootstrap, default
# the latest), WHET_TARGET (override the detected platform).

set -u

DIST="${WHET_DIST_SERVER:-https://github.com/romishalom/keen/releases}"
DIST="${DIST%/}"

say() { printf 'whet-install: %s\n' "$1" >&2; }
err() { say "error: $1"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "need '$1' (command not found)"; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<'USAGE'
whet-install: install keen with whet

USAGE:
    install.sh [-h|--help] [whet init options...]

Everything after the script name is passed to `whet init`, for example:
    -y                          accept the defaults, no prompts
    --default-toolchain <v>     latest (default), a version like 0.3.0, or none
    --profile <default|minimal> keen + completions, or keen only
    --no-modify-path            leave shell profiles alone
    --prefix <dir>              install somewhere other than ~/.keen
    --dist-server <url>         install from a mirror (https://, file://)

Environment: WHET_DIST_SERVER, WHET_VERSION, WHET_TARGET
USAGE
}

detect_target() {
    if [ -n "${WHET_TARGET:-}" ]; then
        printf '%s\n' "$WHET_TARGET"
        return
    fi
    os=$(uname -s 2>/dev/null) || err "cannot run uname"
    arch=$(uname -m 2>/dev/null) || err "cannot run uname"
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) err "unsupported CPU architecture: $arch (build keen from source: cargo install --path .)" ;;
    esac
    case "$os" in
        Linux)
            libc=gnu
            if (ldd --version 2>&1 | grep -qi musl) || ls /lib/ld-musl-* >/dev/null 2>&1; then
                libc=musl
            fi
            printf '%s-unknown-linux-%s\n' "$arch" "$libc"
            ;;
        Darwin)
            # a shell running under Rosetta reports x86_64 on Apple silicon
            if [ "$arch" = x86_64 ] && sysctl -n hw.optional.arm64 2>/dev/null | grep -q 1; then
                arch=aarch64
            fi
            printf '%s-apple-darwin\n' "$arch"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            printf 'x86_64-pc-windows-msvc\n'
            ;;
        *) err "unsupported operating system: $os" ;;
    esac
}

# download URL DEST: https (TLS 1.2+), file://, or http to the local machine only
download() {
    url=$1
    dest=$2
    case "$url" in
        file://*)
            cp "${url#file://}" "$dest" || err "cannot read ${url#file://}"
            return
            ;;
        https://*) ;;
        http://localhost[:/]* | http://127.0.0.1[:/]* | http://\[::1\][:/]*) ;;
        *) err "refusing to download over an insecure or unsupported URL: $url" ;;
    esac
    if have curl; then
        case "$url" in
            https://*) curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 --output "$dest" "$url" || err "download failed: $url" ;;
            *) curl --fail --silent --show-error --location --retry 3 --output "$dest" "$url" || err "download failed: $url" ;;
        esac
    elif have wget; then
        case "$url" in
            https://*) wget --https-only --quiet --tries=3 -O "$dest" "$url" || err "download failed: $url" ;;
            *) wget --quiet --tries=3 -O "$dest" "$url" || err "download failed: $url" ;;
        esac
    else
        err "need 'curl' or 'wget' to download files"
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif have openssl; then
        openssl dgst -sha256 "$1" | sed 's/^.*= //'
    else
        err "need sha256sum, shasum or openssl to verify the download"
    fi
}

main() {
    for arg in "$@"; do
        case "$arg" in
            -h | --help)
                usage
                exit 0
                ;;
        esac
    done
    need uname
    need mkdir
    need chmod
    need rm
    need cut

    target=$(detect_target)
    ext=""
    case "$target" in *windows*) ext=".exe" ;; esac

    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t whet) || err "cannot create a temporary directory"
    trap 'rm -rf "$tmp"' EXIT INT TERM HUP

    version="${WHET_VERSION:-}"
    if [ -z "$version" ]; then
        # GitHub releases latest download endpoint for the 'latest' asset file
        download "$DIST/latest/download/latest" "$tmp/latest"
        version=$(head -n 1 "$tmp/latest" | tr -d '\r\n ')
    fi
    case "$version" in
        '' | *[!0-9A-Za-z.-]*) err "the release server returned an invalid version: '$version'" ;;
    esac

    say "downloading whet $version for $target"
    # GitHub releases flat asset layout: /releases/download/v<version>/<filename>
    url="$DIST/download/v$version/whet-$target$ext"
    download "$url" "$tmp/whet$ext"
    download "$url.sha256" "$tmp/whet.sha256"

    want=$(head -n 1 "$tmp/whet.sha256" | cut -d' ' -f1 | tr 'A-F' 'a-f')
    have_sum=$(sha256_of "$tmp/whet$ext")
    if [ "$want" != "$have_sum" ]; then
        err "checksum mismatch for whet (expected $want, got $have_sum); nothing was installed"
    fi
    chmod +x "$tmp/whet$ext"

    # Prompts need the terminal even when this script arrives through a pipe (curl | sh).
    if [ ! -t 0 ] && [ -r /dev/tty ] && (: </dev/tty) 2>/dev/null; then
        "$tmp/whet$ext" init "$@" </dev/tty
    else
        "$tmp/whet$ext" init "$@"
    fi
    status=$?
    exit "$status"
}

# Everything is wrapped in main() so a truncated download can never run half a script.
main "$@"