#!/usr/bin/env bash

set -euo pipefail

# nvm-rs installer script
# Usage: curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash

REPO_OWNER="mose-x"
REPO_NAME="nvm-rust"
BINARY_NAME="nvm"
INSTALL_DIR="${NVM_INSTALL_DIR:-$HOME/.nvm.rust/bin}"
BIN_LINK="/usr/local/bin/nvm"

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
        Linux)   os="linux" ;;
        Darwin)  os="darwin" ;;
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
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)
            error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac
    echo "$arch"
}

get_latest_version() {
    local latest=""
    if command -v curl >/dev/null 2>&1; then
        latest=$(curl -fsSL "${GITHUB_PREFIX}https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        latest=$(wget -qO- "${GITHUB_PREFIX}https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    else
        error "Neither curl nor wget is installed"
        exit 1
    fi

    if [ -z "$latest" ]; then
        error "Failed to get latest version"
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

main() {
    info "Installing nvm-rs..."

    local os=$(detect_os)
    local arch=$(detect_arch)
    info "Detected OS: $os, Architecture: $arch"

    local version="${NVM_VERSION:-}"
    if [ -z "$version" ]; then
        info "Checking latest version..."
        version=$(get_latest_version)
        success "Latest version: $version"
    else
        info "Using specified version: $version"
    fi

    local target="${arch}-${os}"
    if [ "$os" = "linux" ]; then
        target="${arch}-unknown-linux-gnu"
    elif [ "$os" = "darwin" ]; then
        target="${arch}-apple-darwin"
    fi

    local archive="nvm-${target}.tar.gz"
    local download_url="${GITHUB_DOWNLOAD}/${version}/${archive}"

    info "Downloading $archive..."
    info "URL: $download_url"

    local tmp_dir
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

    mkdir -p "$INSTALL_DIR"
    mv "${tmp_dir}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    success "Installed to ${INSTALL_DIR}/${BINARY_NAME}"

    # Download shell integration scripts
    local nvm_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust}"
    local shell_dir="${nvm_dir}/shell"

    info "Downloading shell integration scripts..."

    local raw_base="https://raw.githubusercontent.com/${REPO_OWNER}/${REPO_NAME}"
    if [ -n "$GITHUB_PREFIX" ]; then
        raw_base="${GITHUB_PREFIX}${raw_base}"
    fi

    # Download nvm.sh (for bash/zsh)
    if ! download_file "${raw_base}/${version}/shell/nvm.sh" "${shell_dir}/nvm.sh" 2>/dev/null; then
        download_file "${raw_base}/main/shell/nvm.sh" "${shell_dir}/nvm.sh" 2>/dev/null || true
    fi

    # Download nvm.fish (for Fish shell)
    if ! download_file "${raw_base}/${version}/shell/nvm.fish" "${shell_dir}/nvm.fish" 2>/dev/null; then
        download_file "${raw_base}/main/shell/nvm.fish" "${shell_dir}/nvm.fish" 2>/dev/null || true
    fi

    # Download nvm.psm1 (for PowerShell)
    if ! download_file "${raw_base}/${version}/shell/nvm.psm1" "${shell_dir}/nvm.psm1" 2>/dev/null; then
        download_file "${raw_base}/main/shell/nvm.psm1" "${shell_dir}/nvm.psm1" 2>/dev/null || true
    fi

    success "Shell integration scripts installed"

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

    local source_line="source ${shell_dir}/nvm.sh"
    local fish_path_line="set -gx PATH ${INSTALL_DIR} \$PATH"

    if [ -f "$shell_profile" ]; then
        if grep -qF "nvm.sh" "$shell_profile" 2>/dev/null || grep -qF "nvm.rust" "$shell_profile" 2>/dev/null; then
            info "Shell integration already configured in $shell_profile"
        else
            echo "" >> "$shell_profile"
            echo "# nvm-rs" >> "$shell_profile"
            case "$(basename "$current_shell")" in
                fish)
                    echo "$fish_path_line" >> "$shell_profile"
                    ;;
                *)
                    echo "$source_line" >> "$shell_profile"
                    ;;
            esac
            success "Added to $shell_profile"
        fi
    fi

    # Try to create symlink to /usr/local/bin
    if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
        ln -sf "${INSTALL_DIR}/${BINARY_NAME}" "$BIN_LINK" 2>/dev/null && \
            success "Symlink created: $BIN_LINK" || \
            warn "Could not create symlink at $BIN_LINK (permission denied)"
    fi

    echo ""
    success "nvm-rs $version installed successfully!"
    echo ""
    info "To get started, restart your shell or run:"
    if [ -n "$shell_profile" ] && [ -f "$shell_profile" ]; then
        echo "  source $shell_profile"
    else
        echo "  source ${shell_dir}/nvm.sh"
    fi
    echo ""
    info "Quick start:"
    echo "  nvm install 20          # Install Node.js 20"
    echo "  nvm use 20             # Switch to Node.js 20"
    echo "  nvm ls                 # List installed versions"
    echo ""
    info "For Fish shell, add to ~/.config/fish/config.fish:"
    echo "  set -gx PATH ${INSTALL_DIR} \$PATH"
    echo "  source ${shell_dir}/nvm.fish"
    echo ""
    info "For PowerShell, add to your profile:"
    echo "  Import-Module ${shell_dir}/nvm.psm1"
    echo ""
    info "For China users, use mirror for faster downloads:"
    echo "  curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | GITHUB_MIRROR=ghproxy bash"
}

main "$@"
