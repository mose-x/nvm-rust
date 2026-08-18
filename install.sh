#!/usr/bin/env bash

set -euo pipefail

# nvm-rs installer script
# Usage: curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash

REPO_OWNER="mose-x"
REPO_NAME="nvm-rust"
BINARY_NAME="nvm"
INSTALL_DIR="${NVM_INSTALL_DIR:-$HOME/.nvm.rust/bin}"
BIN_LINK="/usr/local/bin/nvm"

# Directory the script itself lives in. When the release archive is extracted
# and the user runs `./install.sh`, this is the archive root — which already
# contains the `nvm` binary and `shell/` dir. We use that to install from the
# bundle without any network round-trip (offline install). When piped via
# `curl | bash`, BASH_SOURCE is empty — SCRIPT_DIR stays empty and we fall
# back to the online download path below.
SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
fi

# GitHub mirror for China users
# Set GITHUB_MIRROR=ghproxy or custom URL to use a mirror
GITHUB_PREFIX=""
if [ -n "${GITHUB_MIRROR:-}" ]; then
    if [ "$GITHUB_MIRROR" = "ghproxy" ] || [ "$GITHUB_MIRROR" = "gh-proxy" ]; then
        GITHUB_PREFIX="https://ghproxy.com/"
    else
        GITHUB_PREFIX="$GITHUB_MIRROR"
    fi
fi

GITHUB_API="https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}"
GITHUB_DOWNLOAD="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/download"

if [ -n "$GITHUB_PREFIX" ]; then
    GITHUB_DOWNLOAD="${GITHUB_PREFIX}${GITHUB_DOWNLOAD}"
fi

color_print() {
    local color="$1"
    local msg="$2"
    case "$color" in
        red)    printf "\033[0;31m%s\033[0m\n" "$msg" ;;
        green)  printf "\033[0;32m%s\033[0m\n" "$msg" ;;
        yellow) printf "\033[1;33m%s\033[0m\n" "$msg" ;;
        cyan)   printf "\033[0;36m%s\033[0m\n" "$msg" ;;
        *)      printf "%s\n" "$msg" ;;
    esac
}

info() {
    color_print cyan "[INFO] $*"
}

success() {
    color_print green "[OK] $*"
}

warn() {
    color_print yellow "[WARN] $*"
}

error() {
    color_print red "[ERROR] $*" >&2
}

detect_os() {
    local os=""
    case "$(uname -s)" in
        Linux)
            os="linux"
            # Detect musl libc (Alpine, distroless). `ldd --version` on musl
            # prints "musl libc"; on glibc it prints "ldd (GNU libc)".
            # Some Alpine versions print to stderr, so merge streams.
            if ldd --version 2>&1 | grep -qi "musl"; then
                os="linux-musl"
            fi
            ;;
        Darwin)  os="macos" ;;
        *)
            error "Unsupported OS: $(uname -s)"
            exit 1
            ;;
    esac
    echo "$os"
}

detect_arch() {
    local arch=""
    case "$(uname -m)" in
        x86_64|amd64)   arch="x64" ;;
        aarch64|arm64)  arch="arm64" ;;
        *)
            error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac
    echo "$arch"
}

get_latest_version() {
    local api_url="${GITHUB_PREFIX}https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest"
    local html_url="${GITHUB_PREFIX}https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/latest"
    local latest=""

    # Try GitHub API first
    if command -v curl >/dev/null 2>&1; then
        latest=$(curl -fsSL "$api_url" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        latest=$(wget -qO- "$api_url" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    else
        error "Neither curl nor wget is installed"
        exit 1
    fi

    # Fallback: GitHub API rate-limited (60/hour for anonymous).
    # The releases/latest HTML page 302-redirects to releases/tag/<tag>,
    # which is served by github.com (not api.github.com) and is NOT rate-limited.
    if [ -z "$latest" ]; then
        info "GitHub API unavailable, trying HTML fallback..."
        if command -v curl >/dev/null 2>&1; then
            latest=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$html_url" 2>/dev/null | grep -o '[^/]*$')
        elif command -v wget >/dev/null 2>&1; then
            latest=$(wget -q -O /dev/null --server-response "$html_url" 2>&1 | grep -i 'location:' | tail -1 | grep -o '[^/]*$' | tr -d '[:space:]')
        fi
    fi

    if [ -z "$latest" ]; then
        error "Failed to get latest version (GitHub API rate-limited?). Try: NVM_VERSION=v2.1.2 ./install.sh"
        exit 1
    fi

    echo "$latest"
}

download_file() {
    local url="$1"
    local dest="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$dest"
    else
        error "Neither curl nor wget is installed"
        exit 1
    fi
}

# install_completion — auto-install tab-completion for the user's shell.
#
# Why: `nvm completion <shell>` (see src/completions.rs) is the single source
# of truth for the completion script body. Running it here keeps the generated
# file in lock-step with the installed binary — no stale shipped copy.
#
# Args: $1=current_shell  $2=shell_profile  $3=shell_dir  $4=nvm_dir
#
# Skip rules (all silent, never fail the install):
#   - NVM_NO_COMPLETION=1          user opted out
#   - SHELL unset / unrecognized   csh/tcsh/nushell/xonsh not supported
#   - binary missing               something went wrong
#   - `nvm completion` fails       older binary without the subcommand
#
# Idempotency: greps the rc for the completion path before appending.
install_completion() {
    local current_shell="$1"
    local shell_profile="$2"
    local shell_dir="$3"
    local nvm_dir="$4"

    if [ "${NVM_NO_COMPLETION:-0}" = "1" ]; then
        info "NVM_NO_COMPLETION=1, skipping completion"
        return 0
    fi

    local shell_name
    shell_name=$(basename "${current_shell:-}")
    case "$shell_name" in
        bash|zsh|fish) ;;
        *) return 0 ;;
    esac

    local nvm_bin="${INSTALL_DIR}/${BINARY_NAME}"
    if [ ! -x "$nvm_bin" ]; then
        warn "nvm binary not found at $nvm_bin, skipping completion"
        return 0
    fi

    # Let the binary generate the script file; silence its stdout banner.
    if ! "$nvm_bin" completion "$shell_name" >/dev/null 2>&1; then
        info "nvm completion not available, skipping"
        return 0
    fi

    local completions_dir="${nvm_dir}/completions"
    case "$shell_name" in
        bash)
            local completion_file="${completions_dir}/nvm.bash"
            local source_line="source ${completion_file}"
            if [ -z "$shell_profile" ] || [ ! -f "$shell_profile" ]; then
                info "Completion written: $completion_file"
                info "Add to your shell rc:  $source_line"
                return 0
            fi
            if grep -qF "$completion_file" "$shell_profile" 2>/dev/null; then
                info "Bash completion already configured"
                return 0
            fi
            printf '\n# nvm-rs completion\n%s\n' "$source_line" >> "$shell_profile"
            success "Added bash completion to $shell_profile"
            ;;
        zsh)
            local completion_file="${completions_dir}/_nvm"
            # zsh fpath takes a DIRECTORY, not a file — adding the file path
            # is a common mistake that silently breaks autoload.
            if [ -z "$shell_profile" ] || [ ! -f "$shell_profile" ]; then
                info "Completion written: $completion_file"
                info "Add to ~/.zshrc:"
                echo "  fpath=( ${completions_dir} \$fpath )"
                echo "  autoload -Uz _nvm"
                return 0
            fi
            if grep -qF "$completions_dir" "$shell_profile" 2>/dev/null; then
                info "zsh completion already configured"
                return 0
            fi
            # Bootstrap compinit if missing (filter comment lines first —
            # a `# no compinit` comment must not fool the check).
            local prepend_compinit=""
            if ! grep -vE '^[[:space:]]*#' "$shell_profile" 2>/dev/null | grep -qF 'compinit'; then
                prepend_compinit="autoload -Uz compinit && compinit"$'\n'
            fi
            printf '\n# nvm-rs completion\n%s%s\n' \
                "$prepend_compinit" \
                "fpath=( ${completions_dir} \$fpath )"$'\n'"autoload -Uz _nvm" \
                >> "$shell_profile"
            success "Added zsh completion to $shell_profile"
            ;;
        fish)
            # fish auto-loads by filename from this dir; no rc edit needed.
            local fish_dir="${HOME}/.config/fish/completions"
            mkdir -p "$fish_dir"
            local src="${completions_dir}/nvm.fish"
            if [ -f "$src" ]; then
                cp -f "$src" "${fish_dir}/nvm.fish"
                success "Installed fish completion to ${fish_dir}/nvm.fish"
            else
                warn "Fish completion file not generated"
            fi
            ;;
    esac
}

create_shims() {
    local nvm_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust}"
    local nvm_bin="${nvm_dir}/bin/nvm"

    # If nvm binary exists, delegate to it — it creates all 8 shims
    # (node/npm/npx/corepack/pnpm/pnpx/yarn/yarnpkg) with path traversal
    # protection and Windows cmd.exe metacharacter rejection.
    if [ -x "$nvm_bin" ]; then
        "$nvm_bin" refresh >/dev/null 2>&1 || true
        return
    fi

    # Fallback: inline shim creation (only 4 commands, no security guards).
    # The nvm binary will overwrite these on first `nvm use` / `nvm install`.
    local shims_dir="${nvm_dir}/shims"
    mkdir -p "$shims_dir"

    for cmd in node npm npx corepack; do
        cat > "$shims_dir/$cmd" << 'SHIM_EOF'
#!/bin/sh
NVM_DIR="${NVM_DIR:-$HOME/.nvm.rust}"
CMD=$(basename "$0")
read_current() { cat "$NVM_DIR/current" 2>/dev/null | tr -d "[:space:]"; }
CURRENT=$(read_current)
if [ "$CURRENT" = "none" ]; then
    echo "nvm: deactivated. Run 'nvm use <version>' to reactivate." >&2
    exit 1
fi
if [ -z "$CURRENT" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
    "$NVM_DIR/bin/nvm" auto --silent 2>/dev/null
    CURRENT=$(read_current)
fi
if [ -z "$CURRENT" ] || [ "$CURRENT" = "none" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
    echo "nvm: $CMD not found. Run 'nvm use <version>' or 'nvm install <version>'." >&2
    exit 1
fi
exec "$NVM_DIR/$CURRENT/bin/$CMD" "$@"
SHIM_EOF
        chmod +x "$shims_dir/$cmd"
    done
}

main() {
    info "Installing nvm-rs..."

    local os=$(detect_os)
    local arch=$(detect_arch)
    info "Detected OS: $os, Architecture: $arch"

    local version="${NVM_VERSION:-}"
    local tmp_dir=""
    local source_dir=""
    local offline=0

    # Offline detection: if the script sits next to a bundled `nvm` binary
    # (the user extracted the release archive and ran `./install.sh`), use
    # that binary directly — no GitHub API call, no download. When piped via
    # `curl | bash`, SCRIPT_DIR is empty and we fall through to online mode.
    if [ -n "${SCRIPT_DIR:-}" ] && [ -x "${SCRIPT_DIR}/${BINARY_NAME}" ]; then
        offline=1
        source_dir="$SCRIPT_DIR"
        info "Found bundled binary at $SCRIPT_DIR (offline install)"
        if [ -z "$version" ]; then
            version=$("${SCRIPT_DIR}/${BINARY_NAME}" --version 2>/dev/null || echo "")
            if [ -n "$version" ]; then
                success "Detected version: $version"
            else
                version="unknown"
            fi
        else
            info "Using specified version: $version"
        fi
    else
        # Online mode: fetch latest release tarball from GitHub.
        if [ -z "$version" ]; then
            info "Checking latest version..."
            version=$(get_latest_version)
            success "Latest version: $version"
        else
            info "Using specified version: $version"
        fi

        # Strip leading 'v' from tag (v2.0.0 → 2.0.0) for asset filename.
        # Asset naming: nvm-<version>-<os>-<arch>.tar.gz
        local version_num="${version#v}"
        local archive="nvm-${version_num}-${os}-${arch}.tar.gz"
        local download_url="${GITHUB_DOWNLOAD}/${version}/${archive}"

        info "Downloading $archive..."
        info "URL: $download_url"

        tmp_dir=$(mktemp -d)
        trap 'rm -rf "$tmp_dir"' EXIT

        local archive_path="${tmp_dir}/${archive}"
        if ! download_file "$download_url" "$archive_path"; then
            error "Failed to download $archive"
            exit 1
        fi
        success "Download complete"

        info "Extracting..."
        tar -xzf "$archive_path" -C "$tmp_dir"
        source_dir="$tmp_dir"
    fi

    # Install binary — cp (not mv) so offline mode doesn't damage the
    # extracted bundle; online mode's tmp_dir is cleaned by the trap.
    mkdir -p "$INSTALL_DIR"
    cp -f "${source_dir}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    success "Installed to ${INSTALL_DIR}/${BINARY_NAME}"

    # Create shim scripts for node/npm/npx/corepack so they resolve
    # via the `current` file without shell-wrapper PATH manipulation.
    create_shims

    # Install shell integration scripts shipped inside the tarball.
    # The release archive includes `shell/{nvm.sh,nvm.fish,nvm.psm1}` so we
    # copy them from the local extraction — no extra network round-trip
    # to raw.githubusercontent.com (which would fail behind proxies, on
    # offline machines, or for users who deleted the repo's `main` branch
    # tag). Fall back to a raw download only if the bundled files are
    # missing (e.g. an older / hand-rolled tarball).
    local nvm_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust}"
    local shell_dir="${nvm_dir}/shell"
    mkdir -p "$shell_dir"

    info "Installing shell integration scripts..."
    local bundled_shell="${source_dir}/shell"
    if [ -d "$bundled_shell" ]; then
        cp -f "${bundled_shell}/nvm.sh"   "${shell_dir}/nvm.sh"
        cp -f "${bundled_shell}/nvm.fish" "${shell_dir}/nvm.fish"
        cp -f "${bundled_shell}/nvm.psm1" "${shell_dir}/nvm.psm1"
        # Also copy nvm.sh to bin/ — all Rust code references bin/nvm.sh,
        # and nvm.sh itself has NVM_RUST_SH="${NVM_RUST_DIR}/bin/nvm.sh".
        mkdir -p "${INSTALL_DIR}"
        cp -f "${bundled_shell}/nvm.sh"   "${INSTALL_DIR}/nvm.sh"
        success "Shell integration scripts installed (bundled)"
    elif [ "$offline" = "1" ]; then
        # Offline bundle has no shell/ dir — can't download, just warn.
        warn "Bundle has no shell/ dir; skipping shell integration"
    else
        # Legacy fallback: tarball without bundled shell/ dir.
        warn "Tarball does not contain shell/ dir, falling back to download"
        local raw_base="https://raw.githubusercontent.com/${REPO_OWNER}/${REPO_NAME}"
        if [ -n "$GITHUB_PREFIX" ]; then
            raw_base="${GITHUB_PREFIX}${raw_base}"
        fi
        download_file "${raw_base}/${version}/shell/nvm.sh"   "${shell_dir}/nvm.sh"   2>/dev/null || \
            download_file "${raw_base}/main/shell/nvm.sh"    "${shell_dir}/nvm.sh"   2>/dev/null || true
        # Also copy/download to bin/ for code that references bin/nvm.sh
        cp -f "${shell_dir}/nvm.sh" "${INSTALL_DIR}/nvm.sh" 2>/dev/null || \
            download_file "${raw_base}/main/shell/nvm.sh" "${INSTALL_DIR}/nvm.sh" 2>/dev/null || true
        download_file "${raw_base}/${version}/shell/nvm.fish" "${shell_dir}/nvm.fish" 2>/dev/null || \
            download_file "${raw_base}/main/shell/nvm.fish"  "${shell_dir}/nvm.fish" 2>/dev/null || true
        download_file "${raw_base}/${version}/shell/nvm.psm1" "${shell_dir}/nvm.psm1" 2>/dev/null || \
            download_file "${raw_base}/main/shell/nvm.psm1"  "${shell_dir}/nvm.psm1" 2>/dev/null || true
        success "Shell integration scripts installed (downloaded)"
    fi

    # Detect shell and add to PATH
    local shell_profile=""
    local current_shell="${SHELL:-}"

    case "$(basename "$current_shell")" in
        zsh)
            shell_profile="$HOME/.zshrc"
            ;;
        fish)
            shell_profile="$HOME/.config/fish/config.fish"
            ;;
        bash)
            if [ "$os" = "darwin" ]; then
                shell_profile="$HOME/.bash_profile"
            else
                shell_profile="$HOME/.bashrc"
            fi
            ;;
        *)
            shell_profile="$HOME/.profile"
            ;;
    esac

    local path_line="export PATH=\"${nvm_dir}/shims:${INSTALL_DIR}:\$PATH\""
    local fish_path_line="set -gx PATH ${nvm_dir}/shims ${INSTALL_DIR} \$PATH"

    if [ -f "$shell_profile" ]; then
        if grep -qF "# nvm-rs" "$shell_profile" 2>/dev/null; then
            info "Shell integration already configured in $shell_profile"
        else
            echo "" >> "$shell_profile"
            echo "# nvm-rs" >> "$shell_profile"
            case "$(basename "$current_shell")" in
                fish)
                    echo "$fish_path_line" >> "$shell_profile"
                    ;;
                *)
                    echo "$path_line" >> "$shell_profile"
                    ;;
            esac
            success "Added to $shell_profile"
        fi
    else
        # Fresh install: the rc file doesn't exist yet. Create it so PATH
        # is configured immediately — without this, a brand-new machine
        # would install nvm but `nvm` wouldn't be on PATH until the user
        # manually creates an rc file.
        mkdir -p "$(dirname "$shell_profile")"
        echo "# nvm-rs" >> "$shell_profile"
        case "$(basename "$current_shell")" in
            fish)
                echo "$fish_path_line" >> "$shell_profile"
                ;;
            *)
                echo "$path_line" >> "$shell_profile"
                ;;
        esac
        success "Created $shell_profile with nvm-rs config"
    fi

    # Auto-install tab-completion for the current shell.
    install_completion "$current_shell" "$shell_profile" "$shell_dir" "$nvm_dir"

    # Try to create symlink to /usr/local/bin
    if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
        ln -sf "${INSTALL_DIR}/${BINARY_NAME}" "$BIN_LINK" 2>/dev/null && \
            success "Symlink created: $BIN_LINK" || \
            warn "Could not create symlink at $BIN_LINK (permission denied)"
    fi

    echo ""
    success "nvm-rs $version installed successfully!"
    echo ""
    # A child process cannot `source` into the parent shell, so we print
    # the exact command for the user to copy-paste. This is the closest
    # to "auto-source" that's safe — `exec $SHELL -l` would discard the
    # user's current session state.
    info "To activate now, run:"
    if [ -n "$shell_profile" ] && [ -f "$shell_profile" ]; then
        echo "  source $shell_profile"
    else
        echo "  source ${shell_dir}/nvm.sh"
    fi
    echo ""
    info "Or open a new terminal to apply changes automatically."
    echo ""
    info "Quick start:"
    echo "  nvm install 20          # Install Node.js 20"
    echo "  nvm use 20             # Switch to Node.js 20"
    echo "  nvm ls                 # List installed versions"
    if [ "$offline" = "0" ]; then
        echo ""
        info "For China users, use mirror for faster downloads:"
        echo "  curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | GITHUB_MIRROR=ghproxy bash"
    fi
}

uninstall_self() {
    echo ""
    warn "This will remove nvm itself (binary, nvm.sh, shims, shell config)."
    info "Node versions and config will be preserved at $HOME/.nvm.rust/"
    echo ""
    read -p "Continue? [y/N] " confirm
    if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
        echo "Cancelled."
        exit 0
    fi

    local nvm_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust}"
    rm -f "${nvm_dir}/bin/nvm" "${nvm_dir}/bin/nvm.sh" 2>/dev/null || true
    rm -rf "${nvm_dir}/shims" 2>/dev/null || true
    rm -f "${nvm_dir}/current" 2>/dev/null || true
    rm -f "$BIN_LINK" 2>/dev/null || true
    clean_shell_config

    echo ""
    success "nvm uninstalled. Node versions preserved at $nvm_dir/"
    info "Reinstall: curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash"
}

uninstall_all() {
    echo ""
    warn "This will remove nvm AND ALL installed Node versions."
    info "Everything in $HOME/.nvm.rust/ will be deleted."
    echo ""
    read -p "Continue? [y/N] " confirm
    if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
        echo "Cancelled."
        exit 0
    fi

    local nvm_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust}"
    rm -rf "$nvm_dir" 2>/dev/null || true
    rm -f "$BIN_LINK" 2>/dev/null || true
    clean_shell_config

    echo ""
    success "nvm and all Node versions uninstalled."
}

clean_shell_config() {
    local profile=""
    local current_shell="${SHELL:-}"
    case "$(basename "$current_shell")" in
        zsh)  profile="$HOME/.zshrc" ;;
        fish) profile="$HOME/.config/fish/config.fish" ;;
        bash) if [ "$(uname -s)" = "Darwin" ]; then profile="$HOME/.bash_profile"; else profile="$HOME/.bashrc"; fi ;;
        *)    profile="$HOME/.profile" ;;
    esac
    [ -f "$profile" ] || return 0
    cp "$profile" "${profile}.bak" 2>/dev/null || true
    grep -Ev "nvm.rust|nvm.sh|NVM_HOME" "$profile" > "${profile}.tmp" 2>/dev/null && mv "${profile}.tmp" "$profile" || rm -f "${profile}.tmp"
    info "Shell config cleaned: $profile"
}

# --uninstall support
if [ "${1:-}" = "--uninstall" ]; then
    if [ "${2:-}" = "--self" ]; then
        uninstall_self
    else
        uninstall_all
    fi
    exit 0
fi

main "$@"
