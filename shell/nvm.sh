# nvm.sh — Shell integration for nvm-rs
# Source this file from your ~/.bashrc, ~/.zshrc, or ~/.profile
#
# Usage:
#   Linux/macOS (bash/zsh): source /path/to/nvm.sh
#   Fish: source /path/to/nvm.fish  (or use the Fish module)
#   PowerShell: Import-Module /path/to/nvm.psm1

NVM_RUST_DIR="${NVM_DIR:-$HOME/.nvm.rust}"
NVM_RUST_SH="${NVM_RUST_DIR}/bin/nvm.sh"
NVM_RUST_BIN="${NVM_RUST_DIR}/bin"

# Check if nvm binary exists
_nvm_binary_exists() {
    [ -f "${NVM_RUST_BIN}/nvm" ] || [ -f "${NVM_RUST_BIN}/nvm.exe" ]
}

# Add nvm bin to PATH if not already there
_nvm_prepend_path() {
    case ":${PATH}:" in
        *":${NVM_RUST_BIN}:"*) ;;
        *) export PATH="${NVM_RUST_BIN}:${PATH}" ;;
    esac
}

# Initialize
_nvm_init() {
    if ! _nvm_binary_exists; then
        return 0
    fi

    _nvm_prepend_path

    # Source shell-specific integration if available
    if [ -f "${NVM_RUST_SH}" ]; then
        . "${NVM_RUST_SH}"
    fi

    # Auto-switch on cd (bash/zsh)
    if [ -z "$NVM_RUST_AUTO_SWITCH_DONE" ]; then
        export NVM_RUST_AUTO_SWITCH_DONE=1
        _nvm_auto_switch() {
            if [ -f ".nvmrc" ]; then
                local ver
                ver=$(cat .nvmrc | tr -d '[:space:]')
                if [ -n "$ver" ]; then
                    local current
                    current=$("${NVM_RUST_BIN}/nvm" current 2>/dev/null | tr -d '[:space:]')
                    if [ "$current" != "$ver" ]; then
                        "${NVM_RUST_BIN}/nvm" use "$ver" >/dev/null 2>&1
                    fi
                fi
            fi
        }

        # Hook into PROMPT_COMMAND (bash) or precmd (zsh)
        case "$OSTYPE" in
            darwin*|linux*)
                if [ -n "$BASH_VERSION" ]; then
                    PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}_nvm_auto_switch"
                elif [ -n "$ZSH_VERSION" ]; then
                    autoload -Uz add-zsh-hook
                    add-zsh-hook precmd _nvm_auto_switch
                fi
                ;;
        esac
    fi
}

# Load shell completions if available
_nvm_load_completions() {
    local completions_dir="${NVM_RUST_DIR}/completions"

    case "$OSTYPE" in
        darwin*|linux*)
            if [ -n "$BASH_VERSION" ] && [ -f "${completions_dir}/nvm.bash" ]; then
                . "${completions_dir}/nvm.bash"
            elif [ -n "$ZSH_VERSION" ] && [ -f "${completions_dir}/_nvm" ]; then
                fpath=( "${completions_dir}" $fpath )
                autoload -Uz _nvm
            fi
            ;;
    esac
}

# Wrapper function for nvm command
nvm() {
    if ! _nvm_binary_exists; then
        echo "nvm-rust not found. Install with:"
        echo "  curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash" >&2
        return 1
    fi

    local cmd="${1:-}"

    case "$cmd" in
        use)
            if [ $# -lt 2 ]; then
                echo "Usage: nvm use <version>" >&2
                return 1
            fi
            "${NVM_RUST_BIN}/nvm" "$@"
            _nvm_prepend_path
            ;;
        auto)
            "${NVM_RUST_BIN}/nvm" auto
            ;;
        deactivate)
            export PATH="${PATH#${NVM_RUST_BIN}:}"
            echo "nvm-rust deactivated (PATH updated)"
            ;;
        unload)
            export PATH="${PATH#${NVM_RUST_BIN}:}"
            unset -f nvm _nvm_auto_switch
            echo "nvm-rust unloaded from shell"
            ;;
        shell)
            echo "NVM_RUST_DIR: $NVM_RUST_DIR"
            echo "NVM_RUST_BIN: $NVM_RUST_BIN"
            ;;
        *)
            "${NVM_RUST_BIN}/nvm" "$@"
            ;;
    esac
}

# Auto-initialize
_nvm_init

# Load completions if shell is interactive
if [[ $- == *i* ]] || [ -z "$PS1" ]; then
    _nvm_load_completions
fi
