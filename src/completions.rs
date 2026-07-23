use std::fs;

use crate::i18n::{format_t, T};
use crate::system::get_nvm_dir;
use colored::Colorize;

/// Generate shell completions
pub fn generate_completions(shell: Option<&str>) -> anyhow::Result<()> {
    let shell = shell.unwrap_or("bash");

    match shell.to_lowercase().as_str() {
        "bash" => bash_completions(),
        "zsh" => zsh_completions(),
        "fish" => fish_completions(),
        "powershell" | "pwsh" => powershell_completions(),
        _ => {
            eprintln!("{}", format_t("unsupported_shell", &[shell.to_string()]));
            eprintln!("{}", T("completion_hint"));
            Ok(())
        }
    }
}

fn bash_completions() -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let completions_dir = nvm_dir.join("completions");

    if !completions_dir.exists() {
        fs::create_dir_all(&completions_dir)?;
    }

    let completion_file = completions_dir.join("nvm.bash");
    let script = r#"# nvm bash completion
_nvm_completion() {
    local cur prev words cword
    _init_completion -n=: || return

    local commands="install uninstall remove use list ls ls-remote remote current dir which run exec alias unalias mirror auto deactivate unload install-npm install-yarn install-pnpm reinstall-packages version version-remote cache language lang proxy completion corepack migrate help"
    local options="--lts --latest --lts-newer --lts-old --offline --source --no-gpg-verify --reinstall-packages-from --latest-npm --latest-yarn --latest-pnpm --install-if-missing --save --use-on-cd --filter --sort --page"

    case "$cur" in
        -*)
            COMPREPLY=( $(compgen -W "$options" -- "$cur") )
            ;;
        *)
            if [[ ${#words[@]} -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "$commands" -- "$cur") )
            elif [[ ${#words[@]} -eq 3 ]]; then
                case "${words[2]}" in
                    install|use|uninstall|remove|run|exec|which|alias|unalias|reinstall-packages|install-npm|install-yarn|install-pnpm)
                        COMPREPLY=( $(compgen -W "20 18 16 14 12 lts node stable" -- "$cur") )
                        ;;
                    mirror)
                        COMPREPLY=( $(compgen -W "taobao official npmmirror" -- "$cur") )
                        ;;
                    language|lang)
                        COMPREPLY=( $(compgen -W "en cn" -- "$cur") )
                        ;;
                    proxy)
                        COMPREPLY=( $(compgen -W "on off" -- "$cur") )
                        ;;
                    completion)
                        COMPREPLY=( $(compgen -W "bash zsh fish powershell" -- "$cur") )
                        ;;
                    corepack)
                        COMPREPLY=( $(compgen -W "enable disable status" -- "$cur") )
                        ;;
                    cache)
                        COMPREPLY=( $(compgen -W "dir list clear" -- "$cur") )
                        ;;
                    migrate)
                        COMPREPLY=( $(compgen -W "nvm nvm-windows" -- "$cur") )
                        ;;
                esac
            fi
            ;;
    esac
} && complete -F _nvm_completion nvm
"#;

    fs::write(&completion_file, script)?;
    println!(
        "{} {} {}",
        "✓".green().bold(),
        T("completions_written_bash").green(),
        completion_file.display()
    );
    println!();
    println!("{}", T("add_to_bashrc"));
    println!("  source {}", completion_file.display());
    Ok(())
}

fn zsh_completions() -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let completions_dir = nvm_dir.join("completions");

    if !completions_dir.exists() {
        fs::create_dir_all(&completions_dir)?;
    }

    let completion_file = completions_dir.join("_nvm");
    let script = r#"#compdef nvm

_nvm_commands() {
    local commands
    commands=(
        'install:Install a Node.js version'
        'uninstall:Uninstall a version'
        'remove:Uninstall a version (alias)'
        'use:Switch to a version'
        'list:List installed versions'
        'ls:List installed versions (alias)'
        'ls-remote:List remote versions'
        'remote:List remote versions'
        'current:Show current version'
        'dir:Show NVM paths'
        'which:Show binary path'
        'run:Run with version'
        'exec:Execute command with version'
        'alias:Manage aliases'
        'unalias:Remove alias'
        'mirror:Set download mirror'
        'auto:Auto-switch via .nvmrc'
        'deactivate:Restore PATH'
        'unload:Remove from shell'
        'install-npm:Upgrade npm'
        'install-yarn:Install latest yarn'
        'install-pnpm:Install latest pnpm'
        'reinstall-packages:Migrate packages'
        'version:Show version info'
        'version-remote:Show remote versions'
        'cache:Cache management'
        'language:Set language'
        'lang:Set language (alias)'
        'proxy:Proxy settings'
        'completion:Generate completions'
        'corepack:Enable/disable corepack'
        'migrate:Migrate from nvm-sh or nvm-windows'
        'help:Show help'
    )
    _describe 'command' commands
}

_nvm_install_opts() {
    local opts
    opts=(
        '--lts[Install latest LTS]'
        '--latest[Install latest release]'
        '--lts-newer[Install latest LTS if not already installed]'
        '--offline[Install from cache only]'
        '--source[Compile from source]'
        '--no-gpg-verify[Skip GPG signature verification]'
        '--latest-npm[Upgrade npm after install]'
        '--latest-yarn[Install latest yarn after install]'
        '--latest-pnpm[Install latest pnpm after install]'
        '--reinstall-packages-from=[Migrate packages from version]:ver:'
    )
    _describe 'option' opts
}

_nvm_remote_opts() {
    local opts
    opts=(
        '--lts[Show LTS versions only]'
        '--lts-old[Show older LTS versions (<= 18)]'
        '--filter=[Filter by major version]:pattern:'
        '--sort=[Sort order]:order:(desc asc)'
        '--page=[Page number (1-based)]:page:'
    )
    _describe 'option' opts
}

_nvm() {
    local curcontext="$curcontext" state line
    typeset -A opt_args

    _arguments -C \
        '1: :_nvm_commands' \
        '2:: :->version_or_option' \
        '3:: :->option_value' \
        '*: :->args'

    case $state in
        version_or_option)
            case $line[1] in
                install)
                    _nvm_install_opts
                    ;;
                remote|ls-remote)
                    _nvm_remote_opts
                    ;;
                use|uninstall|remove|run|exec|which|alias|unalias|reinstall-packages|install-npm|install-yarn|install-pnpm)
                    _message 'version'
                    ;;
                mirror)
                    _values 'mirror' 'taobao' 'official' 'npmmirror'
                    ;;
                language|lang)
                    _values 'language' 'en' 'cn'
                    ;;
                proxy)
                    _values 'proxy' 'on' 'off'
                    ;;
                completion)
                    _values 'shell' 'bash' 'zsh' 'fish' 'powershell'
                    ;;
                corepack)
                    _values 'corepack' 'enable' 'disable' 'status'
                    ;;
                cache)
                    _values 'cache action' 'dir' 'list' 'clear'
                    ;;
                migrate)
                    _values 'source' 'nvm' 'nvm-windows'
                    ;;
            esac
            ;;
    esac
}

_nvm "$@"
"#;

    fs::write(&completion_file, script)?;
    println!(
        "{} {} {}",
        "✓".green().bold(),
        T("completions_written_zsh").green(),
        completion_file.display()
    );
    println!();
    println!("{}", T("add_to_zshrc"));
    println!("  fpath=( {} $fpath )", completion_file.display());
    println!("  autoload -Uz _nvm");
    Ok(())
}

fn fish_completions() -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let completions_dir = nvm_dir.join("completions");

    if !completions_dir.exists() {
        fs::create_dir_all(&completions_dir)?;
    }

    let completion_file = completions_dir.join("nvm.fish");
    let script = r#"# nvm fish completion

complete -c nvm -n '__fish_use_subcommand' -a 'install' -d 'Install a Node.js version'
complete -c nvm -n '__fish_use_subcommand' -a 'uninstall' -d 'Uninstall a version'
complete -c nvm -n '__fish_use_subcommand' -a 'remove' -d 'Uninstall (alias)'
complete -c nvm -n '__fish_use_subcommand' -a 'use' -d 'Switch to a version'
complete -c nvm -n '__fish_use_subcommand' -a 'list' -d 'List installed versions'
complete -c nvm -n '__fish_use_subcommand' -a 'ls' -d 'List installed (alias)'
complete -c nvm -n '__fish_use_subcommand' -a 'ls-remote' -d 'List remote versions'
complete -c nvm -n '__fish_use_subcommand' -a 'remote' -d 'List remote versions'
complete -c nvm -n '__fish_use_subcommand' -a 'current' -d 'Show current version'
complete -c nvm -n '__fish_use_subcommand' -a 'dir' -d 'Show NVM paths'
complete -c nvm -n '__fish_use_subcommand' -a 'which' -d 'Show binary path'
complete -c nvm -n '__fish_use_subcommand' -a 'run' -d 'Run with version'
complete -c nvm -n '__fish_use_subcommand' -a 'exec' -d 'Execute with version'
complete -c nvm -n '__fish_use_subcommand' -a 'alias' -d 'Manage aliases'
complete -c nvm -n '__fish_use_subcommand' -a 'unalias' -d 'Remove alias'
complete -c nvm -n '__fish_use_subcommand' -a 'mirror' -d 'Set mirror'
complete -c nvm -n '__fish_use_subcommand' -a 'auto' -d 'Auto-switch via .nvmrc'
complete -c nvm -n '__fish_use_subcommand' -a 'deactivate' -d 'Restore PATH'
complete -c nvm -n '__fish_use_subcommand' -a 'unload' -d 'Remove from shell'
complete -c nvm -n '__fish_use_subcommand' -a 'install-npm' -d 'Upgrade npm'
complete -c nvm -n '__fish_use_subcommand' -a 'install-yarn' -d 'Install latest yarn'
complete -c nvm -n '__fish_use_subcommand' -a 'install-pnpm' -d 'Install latest pnpm'
complete -c nvm -n '__fish_use_subcommand' -a 'reinstall-packages' -d 'Migrate packages'
complete -c nvm -n '__fish_use_subcommand' -a 'version' -d 'Show version info'
complete -c nvm -n '__fish_use_subcommand' -a 'version-remote' -d 'Show remote versions'
complete -c nvm -n '__fish_use_subcommand' -a 'cache' -d 'Cache management'
complete -c nvm -n '__fish_use_subcommand' -a 'language' -d 'Set language'
complete -c nvm -n '__fish_use_subcommand' -a 'lang' -d 'Set language (alias)'
complete -c nvm -n '__fish_use_subcommand' -a 'proxy' -d 'Proxy settings'
complete -c nvm -n '__fish_use_subcommand' -a 'completion' -d 'Generate completions'
complete -c nvm -n '__fish_use_subcommand' -a 'corepack' -d 'Corepack support'
complete -c nvm -n '__fish_use_subcommand' -a 'migrate' -d 'Migrate from nvm-sh or nvm-windows'
complete -c nvm -n '__fish_use_subcommand' -a 'help' -d 'Show help'

complete -c nvm -n '__fish_seen_subcommand_from install' -l lts -d 'Install latest LTS'
complete -c nvm -n '__fish_seen_subcommand_from install' -l latest -d 'Install latest release'
complete -c nvm -n '__fish_seen_subcommand_from install' -l lts-newer -d 'Install latest LTS if not installed'
complete -c nvm -n '__fish_seen_subcommand_from install' -l offline -d 'Install from cache'
complete -c nvm -n '__fish_seen_subcommand_from install' -l source -d 'Compile from source'
complete -c nvm -n '__fish_seen_subcommand_from install' -l no-gpg-verify -d 'Skip GPG verification'
complete -c nvm -n '__fish_seen_subcommand_from install' -l latest-npm -d 'Upgrade npm'
complete -c nvm -n '__fish_seen_subcommand_from install' -l latest-yarn -d 'Install latest yarn'
complete -c nvm -n '__fish_seen_subcommand_from install' -l latest-pnpm -d 'Install latest pnpm'
complete -c nvm -n '__fish_seen_subcommand_from install' -l reinstall-packages-from -d 'Migrate packages from version'

complete -c nvm -n '__fish_seen_subcommand_from use' -l install-if-missing -d 'Install if missing'
complete -c nvm -n '__fish_seen_subcommand_from use' -l save -d 'Persist as default'
complete -c nvm -n '__fish_seen_subcommand_from use' -l use-on-cd -d 'Enable auto-switch on cd'

complete -c nvm -n '__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote' -l lts -d 'LTS versions only'
complete -c nvm -n '__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote' -l lts-old -d 'Older LTS (<= 18)'
complete -c nvm -n '__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote' -l filter -d 'Filter by pattern'
complete -c nvm -n '__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote' -l sort -d 'Sort order'
complete -c nvm -n '__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote' -l page -d 'Page number (1-based)'

complete -c nvm -n '__fish_seen_subcommand_from mirror' -a 'taobao' -d 'npmmirror'
complete -c nvm -n '__fish_seen_subcommand_from mirror' -a 'official' -d 'Official mirror'
complete -c nvm -n '__fish_seen_subcommand_from mirror' -a 'npmmirror' -d 'npmmirror'

complete -c nvm -n '__fish_seen_subcommand_from language; or __fish_seen_subcommand_from lang' -a 'en' -d 'English'
complete -c nvm -n '__fish_seen_subcommand_from language; or __fish_seen_subcommand_from lang' -a 'cn' -d 'Chinese'

complete -c nvm -n '__fish_seen_subcommand_from proxy' -a 'on' -d 'Enable proxy'
complete -c nvm -n '__fish_seen_subcommand_from proxy' -a 'off' -d 'Disable proxy'

complete -c nvm -n '__fish_seen_subcommand_from completion' -a 'bash' -d 'Bash'
complete -c nvm -n '__fish_seen_subcommand_from completion' -a 'zsh' -d 'Zsh'
complete -c nvm -n '__fish_seen_subcommand_from completion' -a 'fish' -d 'Fish'
complete -c nvm -n '__fish_seen_subcommand_from completion' -a 'powershell' -d 'PowerShell'

complete -c nvm -n '__fish_seen_subcommand_from corepack' -a 'enable' -d 'Enable corepack'
complete -c nvm -n '__fish_seen_subcommand_from corepack' -a 'disable' -d 'Disable corepack'
complete -c nvm -n '__fish_seen_subcommand_from corepack' -a 'status' -d 'Show status'

complete -c nvm -n '__fish_seen_subcommand_from cache' -a 'dir' -d 'Show cache directory'
complete -c nvm -n '__fish_seen_subcommand_from cache' -a 'list' -d 'List cached files'
complete -c nvm -n '__fish_seen_subcommand_from cache' -a 'clear' -d 'Clear cache'

complete -c nvm -n '__fish_seen_subcommand_from migrate' -a 'nvm' -d 'From nvm-sh'
complete -c nvm -n '__fish_seen_subcommand_from migrate' -a 'nvm-windows' -d 'From nvm-windows'
"#;

    fs::write(&completion_file, script)?;
    println!(
        "{} {} {}",
        "✓".green().bold(),
        T("completions_written_fish").green(),
        completion_file.display()
    );
    println!();
    println!("{}", T("add_to_fish_config"));
    println!("  source {}", completion_file.display());
    Ok(())
}

fn powershell_completions() -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let completions_dir = nvm_dir.join("completions");

    if !completions_dir.exists() {
        fs::create_dir_all(&completions_dir)?;
    }

    let completion_file = completions_dir.join("nvm.ps1");
    let script = r#"# nvm PowerShell completion

$commands = @(
    'install',
    'uninstall',
    'remove',
    'use',
    'list',
    'ls',
    'ls-remote',
    'remote',
    'current',
    'dir',
    'which',
    'run',
    'exec',
    'alias',
    'unalias',
    'mirror',
    'auto',
    'deactivate',
    'unload',
    'install-npm',
    'install-yarn',
    'install-pnpm',
    'reinstall-packages',
    'version',
    'version-remote',
    'cache',
    'language',
    'lang',
    'proxy',
    'completion',
    'corepack',
    'migrate',
    'help'
)

$options = @(
    '--lts',
    '--latest',
    '--lts-newer',
    '--lts-old',
    '--offline',
    '--source',
    '--no-gpg-verify',
    '--latest-npm',
    '--latest-yarn',
    '--latest-pnpm',
    '--reinstall-packages-from',
    '--install-if-missing',
    '--save',
    '--use-on-cd',
    '--filter',
    '--sort',
    '--page'
)

Register-ArgumentCompleter -CommandName nvm -Native -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    # nvm is a native binary, so -ParameterName (which targets named
    # parameters of a PS function/cmdlet) never fires. -Native hands us
    # the full command AST; we inspect it to decide whether the cursor
    # is at the subcommand position (first positional arg) or later.
    $elements = $commandAst.CommandElements
    $atSubcommand = $elements.Count -le 2 -and -not $wordToComplete.StartsWith('-')

    if ($atSubcommand) {
        $commands | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
            [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
        }
    } else {
        $options | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
            [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
        }
    }
}
"#;

    fs::write(&completion_file, script)?;
    println!(
        "{} {} {}",
        "✓".green().bold(),
        T("completions_written_powershell").green(),
        completion_file.display()
    );
    println!();
    println!("{}", T("add_to_powershell_profile"));
    println!("  . {}", completion_file.display());
    Ok(())
}
