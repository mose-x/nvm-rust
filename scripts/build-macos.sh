#!/usr/bin/env bash
# ==========================================================================
# nvm-rust macOS build script
#
# Usage:
#   ./scripts/build-macos.sh              — debug build
#   ./scripts/build-macos.sh release       — release build (optimized + LTO)
#   ./scripts/build-macos.sh check         — fmt + clippy + test (full verification)
#   ./scripts/build-macos.sh check quick    — fmt + clippy only
#
# Prerequisites:
#   - Rust toolchain (rustup): https://rustup.rs/
#   - Xcode Command Line Tools: xcode-select --install
#
# This is a LOCAL build script. CI uses .github/workflows/ci.yml and
# release.yml which have their own cross-compilation, caching, and
# artifact staging — NOT replaced by this script.
# ==========================================================================
set -euo pipefail

cd "$(dirname "$0")/.."

# --- Verify Rust toolchain ---
if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Rust not found. Install: https://rustup.rs/"
    exit 1
fi

# --- Verify Xcode CLT (provides linker) ---
if ! xcode-select -p &> /dev/null; then
    echo "[WARN] Xcode Command Line Tools not found."
    echo "       Install: xcode-select --install"
fi

# --- Set NVM_DIR to temp (avoid test interference) ---
export NVM_DIR="${NVM_DIR:-$(mktemp -d)}"
mkdir -p "$NVM_DIR" 2>/dev/null || true

# --- Detect arch ---
ARCH=$(uname -m)
echo "[INFO] Detected arch: $ARCH"

# --- Dispatch by mode ---
MODE="${1:-build}"

case "$MODE" in
    check)
        echo "[1/3] Formatting..."
        cargo fmt
        cargo fmt --check || { echo "[FAIL] fmt"; exit 1; }
        echo "[OK] Formatting clean."

        echo "[2/3] Clippy..."
        cargo clippy --all-targets -- -D warnings || { echo "[FAIL] clippy"; exit 1; }
        echo "[OK] Clippy clean."

        if [ "${2:-}" != "quick" ]; then
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
        ;;
    build|"")
        echo "[INFO] Building debug..."
        cargo build
        echo "[OK] Debug build: target/debug/nvm"
        ;;
    *)
        echo "Usage: $0 [build|release|check [quick]]"
        exit 1
        ;;
esac
