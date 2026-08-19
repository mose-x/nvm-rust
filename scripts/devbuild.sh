#!/bin/bash
# devbuild.sh — build debug binary and auto-copy to system path or ~/.nvm.rust/bin/
# Usage: ./scripts/devbuild.sh  or  bash scripts/devbuild.sh
#
# Tries system path (/usr/local/bin) first (EDR-safe: real binary in
# system path, symlink in user path). Falls back to user path if no sudo.
set -e
cargo build

SYSTEM_BIN="/usr/local/bin/nvm"
USER_BIN="$HOME/.nvm.rust/bin/nvm"

mkdir -p "$HOME/.nvm.rust/bin"

if [ -f target/debug/nvm ]; then
    # Try system path first (EDR-safe)
    if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
        cp target/debug/nvm "$SYSTEM_BIN"
        chmod +x "$SYSTEM_BIN"
        ln -sf "$SYSTEM_BIN" "$USER_BIN"
        echo "✓ Copied to $SYSTEM_BIN (system path, EDR-safe)"
    elif command -v sudo &>/dev/null && [ -d "/usr/local/bin" ]; then
        sudo cp target/debug/nvm "$SYSTEM_BIN"
        sudo chmod +x "$SYSTEM_BIN"
        ln -sf "$SYSTEM_BIN" "$USER_BIN"
        echo "✓ Copied to $SYSTEM_BIN (system path via sudo, EDR-safe)"
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
