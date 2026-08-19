#!/usr/bin/env bash
# ==========================================================================
# nvm-rust Linux build script
#
# Usage:
#   ./scripts/build-linux.sh                          — debug build
#   ./scripts/build-linux.sh release                   — release build (optimized + LTO)
#   ./scripts/build-linux.sh check                     — fmt + clippy + test
#   ./scripts/build-linux.sh check quick               — fmt + clippy only
#   ./scripts/build-linux.sh release --version 9.9.9  — release with custom version
#
# --version X.Y.Z overrides Cargo.toml version (useful for local builds
# that shouldn't trigger `nvm upgrade` checks). Cargo.toml is restored
# after the build.
#
# Prerequisites:
#   - Rust toolchain (rustup): https://rustup.rs/
#   - For musl builds: musl-tools (sudo apt-get install musl-tools)
# ==========================================================================
set -euo pipefail

cd "$(dirname "$0")/.."

# --- Verify Rust toolchain ---
if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Rust not found. Install: https://rustup.rs/"
    exit 1
fi

# --- Set NVM_DIR to temp (avoid test interference) ---
export NVM_DIR="${NVM_DIR:-$(mktemp -d)}"
mkdir -p "$NVM_DIR" 2>/dev/null || true

# --- Detect musl (for static binary builds) ---
IS_MUSL=false
if ldd --version 2>&1 | grep -qi "musl"; then
    IS_MUSL=true
    echo "[INFO] Detected musl libc (Alpine/distroless). Static binary will be produced."
fi

# --- Parse --version from args ---
CUSTOM_VERSION=""
ARGS=()
for arg in "$@"; do
    if [ "$arg" = "--version" ]; then
        shift_next=true
    elif [ "${shift_next:-}" = "true" ]; then
        CUSTOM_VERSION="$arg"
        shift_next=false
    else
        ARGS+=("$arg")
    fi
done

# --- Override Cargo.toml version if requested ---
if [ -n "$CUSTOM_VERSION" ]; then
    echo "[INFO] Using custom version: $CUSTOM_VERSION"
    ORIG_CARGO=$(cat Cargo.toml)
    trap 'echo "$ORIG_CARGO" > Cargo.toml; echo "[INFO] Restored Cargo.toml"' EXIT
    sed -i "s/^version = \".*\"/version = \"$CUSTOM_VERSION\"/" Cargo.toml
fi

# --- Dispatch by mode ---
MODE="${ARGS[0]:-build}"

case "$MODE" in
    check)
        echo "[1/3] Formatting..."
        cargo fmt
        cargo fmt --check || { echo "[FAIL] fmt"; exit 1; }
        echo "[OK] Formatting clean."

        echo "[2/3] Clippy..."
        cargo clippy --all-targets -- -D warnings || { echo "[FAIL] clippy"; exit 1; }
        echo "[OK] Clippy clean."

        if [ "${ARGS[1]:-}" != "quick" ]; then
            echo "[3/3] Tests..."
            cargo test --all || { echo "[FAIL] tests"; exit 1; }
            echo "[OK] All tests passed."
        else
            echo "[SKIP] Tests skipped (quick mode)."
        fi
        echo ""
        echo "===================================="
        echo "  All checks passed. Ready to commit."
        echo "===================================="
        ;;
    release)
        echo "[INFO] Building release (optimized + LTO)..."
        cargo build --release
        echo "[OK] Release build: target/release/nvm"
        _copy_binary release
        ;;
    build|"")
        echo "[INFO] Building debug..."
        cargo build
        echo "[OK] Debug build: target/debug/nvm"
        _copy_binary debug
        ;;
    *)
        echo "Usage: $0 [build|release|check [quick]] [--version X.Y.Z]"
        exit 1
        ;;
esac

_copy_binary() {
    local mode="$1"
    local src="target/${mode}/nvm"
    local install_dir="${NVM_INSTALL_DIR:-$HOME/.nvm.rust/bin}"
    if [ -f "$src" ]; then
        mkdir -p "$install_dir"
        cp "$src" "$install_dir/nvm"
        chmod +x "$install_dir/nvm"
        echo "[OK] Copied to $install_dir/nvm"
    fi
}
