#!/usr/bin/env bash
# ==========================================================================
# nvm-rust Linux build script
#
# Usage:
#   ./scripts/build-linux.sh              — debug build
#   ./scripts/build-linux.sh release       — release build (optimized + LTO)
#   ./scripts/build-linux.sh check         — fmt + clippy + test (full verification)
#   ./scripts/build-linux.sh check quick    — fmt + clippy only
#
# Prerequisites:
#   - Rust toolchain (rustup): https://rustup.rs/
#   - For musl builds: musl-tools (sudo apt-get install musl-tools)
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

# --- Set NVM_DIR to temp (avoid test interference) ---
export NVM_DIR="${NVM_DIR:-$(mktemp -d)}"
mkdir -p "$NVM_DIR" 2>/dev/null || true

# --- Detect musl (for static binary builds) ---
IS_MUSL=false
if ldd --version 2>&1 | grep -qi "musl"; then
    IS_MUSL=true
    echo "[INFO] Detected musl libc (Alpine/distroless). Static binary will be produced."
fi

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
