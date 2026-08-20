#!/bin/bash
# build.sh — build nvm from source and auto-copy to ~/.nvm.rust/bin/ or /usr/local/bin/
#
# Use this if you downloaded the source and want to build locally.
# If you just want to install (no Rust toolchain needed), use: ./install.sh
#
# Usage: ./build.sh  or  bash build.sh
#
# Simple writability check: tries /usr/local/bin directly. No auto-sudo.
# Falls back to user path if not writable.
set -e
cargo build

USER_BIN="$HOME/.nvm.rust/bin/nvm"

mkdir -p "$HOME/.nvm.rust/bin"

if [ -f target/debug/nvm ]; then
    # Simple writability check: try /usr/local/bin directly
    if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
        cp -f target/debug/nvm "/usr/local/bin/nvm"
        chmod +x "/usr/local/bin/nvm"
        ln -sf "/usr/local/bin/nvm" "$USER_BIN"
        echo "✓ Copied to /usr/local/bin/nvm (system path, EDR-safe)"
    elif command -v sudo &>/dev/null && [ -d "/usr/local/bin" ]; then
        # Suggest sudo (no auto-sudo); fall back to user dir
        echo "⚠ Run for system path: sudo cp target/debug/nvm /usr/local/bin/nvm && sudo ln -sf /usr/local/bin/nvm $USER_BIN"
        cp target/debug/nvm "$USER_BIN"
        chmod +x "$USER_BIN"
        echo "✓ Copied to ~/.nvm.rust/bin/nvm (user path fallback)"
    else
        cp target/debug/nvm "$USER_BIN"
        chmod +x "$USER_BIN"
        echo "✓ Copied to ~/.nvm.rust/bin/nvm (user path — EDR may block)"
    fi
elif [ -f target/debug/nvm.exe ]; then
    cp target/debug/nvm.exe "$HOME/.nvm.rust/bin/nvm.exe"
    echo "✓ Copied to ~/.nvm.rust/bin/nvm.exe"
else
    echo "⚠ Binary not found in target/debug/"
    exit 1
fi
