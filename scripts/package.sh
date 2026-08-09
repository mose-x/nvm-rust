#!/usr/bin/env bash
# ==========================================================================
# nvm-rust package script — creates a release-ready archive.
#
# Used by BOTH:
#   - Local developers: after `scripts/build-PLATFORM.sh release`, run this
#   - CI (release.yml): after cross-compile build, run this
#
# Single source of truth for the "friendly-pack" layout: binary at root,
# README + LICENSE + install scripts + shell integration files beside it.
#
# Usage:
#   scripts/package.sh <binary_path> <version> <os> <arch> [output_dir]
#
# Examples:
#   scripts/package.sh target/release/nvm 2.1.0 linux x64
#     → nvm-2.1.0-linux-x64.tar.gz
#   scripts/package.sh target/release/nvm.exe 2.1.0 windows x64
#     → nvm-2.1.0-windows-x64.zip
#   scripts/package.sh target/x86_64-unknown-linux-musl/release/nvm 2.1.0 linux-musl x64
#     → nvm-2.1.0-linux-musl-x64.tar.gz
#
# Archive format: .tar.gz for Unix (linux/macos), .zip for Windows.
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Args ---
BINARY="${1:?Usage: package.sh <binary_path> <version> <os> <arch> [output_dir]}"
VERSION="${2:?Missing version}"
OS="${3:?Missing os (e.g. linux, linux-musl, macos, windows)}"
ARCH="${4:?Missing arch (e.g. x64, arm64)}"
OUTPUT_DIR="${5:-.}"

if [ ! -f "$BINARY" ]; then
    echo "[ERROR] Binary not found: $BINARY"
    exit 1
fi

# --- Determine archive format ---
if [ "$OS" = "windows" ]; then
    EXT="zip"
else
    EXT="tar.gz"
fi

ASSET_NAME="nvm-${VERSION}-${OS}-${ARCH}.${EXT}"

# --- Stage directory (inside project root for CI workspace compatibility) ---
STAGE="$PROJECT_ROOT/.stage-pack-$$"
rm -rf "$STAGE"
mkdir -p "$STAGE"

echo "[INFO] Staging friendly-pack files..."

# Binary at root
cp "$BINARY" "$STAGE/$(basename "$BINARY")"

# Documentation
cp "$PROJECT_ROOT/README.md" "$STAGE/README.md"
cp "$PROJECT_ROOT/README.ZH_CN.md" "$STAGE/README.ZH_CN.md"
cp "$PROJECT_ROOT/LICENSE" "$STAGE/LICENSE"

# Install scripts
cp "$PROJECT_ROOT/install.sh" "$STAGE/install.sh"
cp "$PROJECT_ROOT/install.ps1" "$STAGE/install.ps1"

# Shell integration
mkdir -p "$STAGE/shell"
cp "$PROJECT_ROOT/shell/nvm.sh" "$STAGE/shell/nvm.sh"
cp "$PROJECT_ROOT/shell/nvm.fish" "$STAGE/shell/nvm.fish"
cp "$PROJECT_ROOT/shell/nvm.psm1" "$STAGE/shell/nvm.psm1"

echo "[INFO] Staged files:"
( cd "$STAGE" && find . -maxdepth 3 -type f | sort )

# --- Create archive ---
echo "[INFO] Creating $ASSET_NAME..."
mkdir -p "$OUTPUT_DIR"

if [ "$EXT" = "zip" ]; then
    ( cd "$STAGE" && 7z a "$OUTPUT_DIR/$ASSET_NAME" . > /dev/null 2>&1 || \
      ( cd "$STAGE" && zip -r "$OUTPUT_DIR/$ASSET_NAME" . > /dev/null 2>&1 ) )
else
    tar -czf "$OUTPUT_DIR/$ASSET_NAME" -C "$STAGE" .
fi

echo "[OK] Created: $OUTPUT_DIR/$ASSET_NAME"

# --- Verify ---
echo "[INFO] Archive contents:"
if [ "$EXT" = "zip" ]; then
    7z l "$OUTPUT_DIR/$ASSET_NAME" 2>/dev/null | tail -n +20 | head -n -2 | awk '{print $NF}' | sort || \
      unzip -l "$OUTPUT_DIR/$ASSET_NAME" 2>/dev/null | tail -n +4 | head -n -2 | awk '{print $NF}' | sort
else
    tar -tzf "$OUTPUT_DIR/$ASSET_NAME" | sort
fi

# --- Cleanup staging ---
rm -rf "$STAGE"

echo "$ASSET_NAME"
