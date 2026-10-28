#!/bin/sh
# Install ask-secret from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/HarshaSurampudi/ask-secret/main/install.sh | sh
#
# Settings (environment variables):
#   ASK_SECRET_VERSION       a release tag such as v0.2.0 (default: the latest release)
#   ASK_SECRET_INSTALL_DIR   where the binaries go (default: ~/.local/bin)
#   ASK_SECRET_NO_SETUP=1    don't offer to set up coding agents afterwards
#   ASK_SECRET_DOWNLOAD_URL  base URL for the release files (for testing mirrors)

set -eu

REPO="HarshaSurampudi/ask-secret"

say() { printf '%s\n' "$*"; }
fail() { printf 'ask-secret install: %s\n' "$*" >&2; exit 1; }

# Everything runs inside main, so a download cut off halfway can't run half a script.
main() {
    need curl
    need tar
    need uname

    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Darwin) target="macos-universal" ;;
        Linux)
            case "$arch" in
                x86_64 | amd64) target="linux-x86_64" ;;
                aarch64 | arm64) target="linux-aarch64" ;;
                *) fail "no Linux build for $arch yet; build from source: https://github.com/$REPO" ;;
            esac
            ;;
        *) fail "no build for $os yet (macOS and Linux only); see https://github.com/$REPO" ;;
    esac

    version="${ASK_SECRET_VERSION:-latest}"
    if [ -n "${ASK_SECRET_DOWNLOAD_URL:-}" ]; then
        base="$ASK_SECRET_DOWNLOAD_URL"
    elif [ "$version" = "latest" ]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$version"
    fi
    dir="${ASK_SECRET_INSTALL_DIR:-$HOME/.local/bin}"
    file="ask-secret-$target.tar.gz"

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "Downloading $file ($version)"
    curl -fsSL "$base/$file" -o "$tmp/$file" || fail "download failed: $base/$file"
    curl -fsSL "$base/$file.sha256" -o "$tmp/$file.sha256" || fail "checksum download failed"
    verify "$tmp/$file" "$(cut -d' ' -f1 < "$tmp/$file.sha256")"

    mkdir -p "$tmp/pkg"
    tar -xzf "$tmp/$file" -C "$tmp/pkg"

    previous=""
    if [ -x "$dir/ask-secret" ]; then
        previous=$("$dir/ask-secret" --version 2>/dev/null || true)
    fi
    mkdir -p "$dir"
    for bin in ask-secret ask-secret-ui; do
        [ -f "$tmp/pkg/$bin" ] || continue
        # Write beside the target and rename, so a running copy is never half-replaced.
        cp "$tmp/pkg/$bin" "$dir/.$bin.new"
        chmod 755 "$dir/.$bin.new"
        mv -f "$dir/.$bin.new" "$dir/$bin"
        if [ "$os" = "Darwin" ]; then
            xattr -d com.apple.quarantine "$dir/$bin" 2>/dev/null || true
        fi
    done

    installed=$("$dir/ask-secret" --version 2>/dev/null) || fail "installed, but $dir/ask-secret doesn't run on this machine"
    if [ -n "$previous" ] && [ "$previous" != "$installed" ]; then
        say "Upgraded: $previous -> $installed ($dir)"
    else
        say "Installed $installed in $dir"
    fi

    on_path=1
    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            on_path=0
            say ""
            say "$dir is not on your PATH. Add this to your shell profile (~/.zshrc, ~/.bashrc):"
            say "  export PATH=\"$dir:\$PATH\""
            ;;
    esac
    if [ "$on_path" = 1 ]; then
        found=$(command -v ask-secret || true)
        if [ -n "$found" ] && [ "$found" != "$dir/ask-secret" ]; then
            say "Note: another ask-secret at $found comes first on your PATH."
        fi
    fi

    offer_setup
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "needs $1"
}

verify() {
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$1" | cut -d' ' -f1)
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$1" | cut -d' ' -f1)
    else
        fail "needs sha256sum or shasum to check the download"
    fi
    [ "$actual" = "$2" ] || fail "checksum mismatch for $(basename "$1"); not installing"
}

# `ask-secret setup` shows what it would change in Claude Code / Codex and asks first.
# Under `curl | sh` our stdin is the script, so it talks to the terminal directly.
offer_setup() {
    if [ "${ASK_SECRET_NO_SETUP:-}" = "1" ]; then
        return
    fi
    if ! (exec </dev/tty) 2>/dev/null; then
        say ""
        say "Next: run \`ask-secret setup\` to teach your coding agents to use it."
        return
    fi
    say ""
    printf 'Set up your coding agents (Claude Code, Codex) to use ask-secret? [Y/n] '
    read -r answer </dev/tty || answer="n"
    case "$answer" in
        n | N | no | No) say "Skipped. Run \`ask-secret setup\` any time." ;;
        *) "$dir/ask-secret" setup </dev/tty || true ;;
    esac
}

main "$@"
