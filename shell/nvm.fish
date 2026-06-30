# nvm.fish — Fish shell integration for nvm-rs
# Place this file in ~/.config/fish/functions/nvm.fish
# Or add: source /path/to/nvm.fish in ~/.config/fish/config.fish

set -g NVM_RUST_DIR "$HOME/.nvm.rust"
set -g NVM_RUST_BIN "$NVM_RUST_DIR/bin"

# Add nvm binaries to PATH
if not contains "$NVM_RUST_BIN" $PATH
    set -gx PATH "$NVM_RUST_BIN" $PATH
end

# Resolve nvm version from alias
function __nvm_resolve --description "Resolve nvm version alias"
    set -l ver "$argv[1]"
    set -l nvm "$NVM_RUST_BIN/nvm"

    if test -z "$ver"
        echo ""
        return 1
    end

    # If it's already a valid version directory, return as-is
    if test -d "$NVM_RUST_DIR/$ver"
        echo "$ver"
        return 0
    end

    # Try to resolve via nvm alias
    set -l resolved (eval "$nvm" alias "$ver" 2>/dev/null | string trim)
    if test -n "$resolved"
        echo "$resolved"
        return 0
    end

    # Return original if no resolution
    echo "$ver"
end

# nvm use — switch Node.js version
function nvm --description "Node version manager (nvm-rust)"
    set -l cmd "$argv[1]"
    set -l ver "$argv[2]"

    switch "$cmd"
        case use
            if test -z "$ver"
                echo "Usage: nvm use <version>"
                return 1
            end
            eval "$NVM_RUST_BIN/nvm" use "$ver"

        case install
            eval "$NVM_RUST_BIN/nvm" install $argv[2..-1]

        case uninstall
            eval "$NVM_RUST_BIN/nvm" uninstall "$ver"

        case ls list
            eval "$NVM_RUST_BIN/nvm" list

        case 'ls-remote' remote
            eval "$NVM_RUST_BIN/nvm" remote $argv[2..-1]

        case current
            eval "$NVM_RUST_BIN/nvm" current

        case which
            eval "$NVM_RUST_BIN/nvm" which "$ver"

        case run
            eval "$NVM_RUST_BIN/nvm" run $argv[2..-1]

        case exec
            eval "$NVM_RUST_BIN/nvm" exec $argv[2..-1]

        case alias
            if test -z "$ver"
                eval "$NVM_RUST_BIN/nvm" alias
            else
                eval "$NVM_RUST_BIN/nvm" alias $argv[2..-1]
            end

        case unalias
            eval "$NVM_RUST_BIN/nvm" unalias "$ver"

        case auto
            eval "$NVM_RUST_BIN/nvm" auto

        case deactivate
            eval "$NVM_RUST_BIN/nvm" deactivate

        case unload
            set -e NVM_RUST_DIR
            set -e NVM_RUST_BIN
            functions -e nvm
            functions -e __nvm_resolve

        case help ''
            echo "nvm-rs — Node.js version manager (Fish shell)"
            echo ""
            echo "Usage: nvm <command> [options]"
            echo ""
            echo "Commands:"
            echo "  use <version>      Switch to a version"
            echo "  install <version>  Install a version"
            echo "  uninstall <ver>   Uninstall a version"
            echo "  ls, list          List installed versions"
            echo "  ls-remote          List remote versions"
            echo "  current            Show current version"
            echo "  which <version>   Show binary path"
            echo "  run <ver> [args]  Run with version"
            echo "  exec <ver> [cmd]  Execute with version"
            echo "  alias [name] [ver] Manage aliases"
            echo "  unalias <name>    Remove alias"
            echo "  auto               Auto-switch via .nvmrc"
            echo "  deactivate         Restore PATH"
            echo "  unload             Remove from shell"
            echo "  cache <sub>        Cache management"
            echo "  language [en|cn]   Set language"
            echo "  proxy [on|off]     Proxy settings"
            echo "  completion <shell> Generate completions"
            echo "  corepack <action>  Corepack support"
            echo ""
            echo "Examples:"
            echo "  nvm install 20"
            echo "  nvm use 20"
            echo "  nvm ls"

        case '*'
            eval "$NVM_RUST_BIN/nvm" $argv
    end
end

# Auto-switch when entering a directory with .nvmrc
function __nvm_auto_switch --on-variable PWD --description "Auto-switch Node.js version when changing directories"
    if test -f ".nvmrc"
        set -l ver (cat .nvmrc | string trim)
        if test -n "$ver"
            # Only switch if different from current
            set -l current ($NVM_RUST_BIN/nvm current 2>/dev/null | string trim)
            if test "$current" != "$ver"
                nvm use "$ver" >/dev/null 2>&1
            end
        end
    end
end
