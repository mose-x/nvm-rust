#!/bin/bash
# devbuild.sh — build debug binary and auto-copy to ~/.nvm.rust/bin/
# Usage: ./scripts/devbuild.sh  or  bash scripts/devbuild.sh
set -e
cargo build
mkdir -p "$HOME/.nvm.rust/bin"
if [ -f target/debug/nvm ]; then
    cp target/debug/nvm "$HOME/.nvm.rust/bin/nvm"
    chmod +x "$HOME/.nvm.rust/bin/nvm"
    echo "✓ Copied to ~/.nvm.rust/bin/nvm"
elif [ -f target/debug/nvm.exe ]; then
    cp target/debug/nvm.exe "$HOME/.nvm.rust/bin/nvm.exe"
    echo "✓ Copied to ~/.nvm.rust/bin/nvm.exe"
else
    echo "⚠ Binary not found in target/debug/"
    exit 1
fi
