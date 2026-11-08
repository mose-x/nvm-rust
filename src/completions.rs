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

/// Silently regenerate every installed shell completion script after a
/// language switch.
///
/// zsh/fish carry description text and follow the new language; bash/powershell
/// have no descriptions but are rewritten too so all four stay in sync (e.g.
/// when commands/options are added later). Only overwrites files that already
/// exist -- never creates files when the user has not installed completions.
/// Silent: does not print "written" / "add to rc" banners, to avoid polluting
/// `nvm language` output. The rewritten file uses the current language's T()
/// descriptions and takes effect the next time a new shell is opened.
///
/// Returns `zsh_regenerated`: whether the zsh completion file was actually
/// rewritten. The caller uses this to decide whether to prompt the user to
/// refresh the current shell -- zsh caches the completion function in the
/// shell process memory, so editing the file does not take effect in the
/// current shell; `unfunction _nvm` followed by a fresh autoload is required.
/// bash/fish/powershell re-read the file on every completion, so no prompt.
pub fn regenerate_completions_if_installed() -> anyhow::Result<bool> {
    let completions_dir = get_nvm_dir().join("completions");
    // Only overwrite files that already exist -- never create files when the
    // user has not installed completions. bash/powershell have no descriptions
    // but are rewritten too to keep all four in sync (e.g. when commands/options
    // are added later).
    let bash = completions_dir.join("nvm.bash");
    if bash.exists() {
        fs::write(&bash, build_bash_script())?;
    }
    let zsh = completions_dir.join("_nvm");
    let mut zsh_regenerated = false;
    if zsh.exists() {
        fs::write(&zsh, build_zsh_script())?;
        zsh_regenerated = true;
    }
    let fish = completions_dir.join("nvm.fish");
    if fish.exists() {
        fs::write(&fish, build_fish_script())?;
    }
    let ps1 = completions_dir.join("nvm.ps1");
    if ps1.exists() {
        fs::write(&ps1, build_powershell_script())?;
    }
    Ok(zsh_regenerated)
}

/// Shared body of the four `*_completions` functions: ensure the completions
/// dir exists, write the script, and print the "written + how to install"
/// banner. Only the script content, the file name, the i18n keys, and the
/// shell-specific install instructions vary — extracting this removes 4× the
/// identical `create_dir_all` / `fs::write` / banner boilerplate (and the
/// TOCTOU comment that was duplicated four times).
///
/// `install_lines` receives the final `completion_file` path and returns the
/// indented instruction line(s) to print after the "add to your rc" message.
/// Most shells emit a single `source`/`.` line; zsh emits two (`fpath=...`
/// plus `autoload`). The caller owns the formatting so the helper stays
/// agnostic to each shell's sourcing convention.
///
/// `note` is an optional plain (non-indented) message printed between the
/// "add to your rc" header and the indented install lines. zsh uses it to
/// warn when `compinit` is missing from ~/.zshrc.
fn write_completion(
    filename: &str,
    script: &str,
    written_key: &str,
    add_to_rc_key: &str,
    note: Option<&str>,
    install_lines: impl Fn(&std::path::Path) -> Vec<String>,
) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let completions_dir = nvm_dir.join("completions");
    // Direct `create_dir_all` (idempotent) instead of `if !exists() { ... }`:
    // the two-step form is a TOCTOU — another process could remove the dir
    // between the stat and the mkdir. Matches `system::ensure_nvm_dir`.
    fs::create_dir_all(&completions_dir)?;

    let completion_file = completions_dir.join(filename);
    fs::write(&completion_file, script)?;
    println!(
        "{} {} {}",
        "✓".green().bold(),
        T(written_key).green(),
        completion_file.display()
    );
    println!();
    println!("{}", T(add_to_rc_key));
    if let Some(n) = note {
        println!("{}", n.yellow());
    }
    for line in install_lines(&completion_file) {
        println!("  {line}");
    }
    Ok(())
}

/// Append a zsh `_<name>_opts` function to `s`. Each option renders as
/// `'flag[translated_desc]suffix'` — `suffix` carries zsh value-completion
/// tags (e.g. `:order:(desc asc)`) which are shell syntax, not translatable.
fn zsh_append_opts(s: &mut String, name: &str, opts: &[(&str, &str, &str)]) {
    s.push_str(name);
    s.push_str("_opts() {\n    local opts\n    opts=(\n");
    for (flag, suffix, key) in opts {
        s.push_str(&format!("        '{}[{}]{}'\n", flag, T(key), suffix));
    }
    s.push_str("    )\n    _describe 'option' opts\n}\n\n");
}

/// Append fish option-completion lines to `s`. Each entry is
/// `(fish_condition, flag, i18n_key)` →
/// `complete -c nvm -n '<condition>' -l <flag> -d '<translated_desc>'`.
fn fish_append_opts(s: &mut String, opts: &[(&str, &str, &str)]) {
    for (cond, flag, key) in opts {
        s.push_str(&format!(
            "complete -c nvm -n '{}' -l {} -d '{}'\n",
            cond,
            flag,
            T(key)
        ));
    }
}

/// Append fish value-completion lines to `s`. Each entry is
/// `(value, i18n_key)` →
/// `complete -c nvm -n '<cond>' -a <value> -d '<translated_desc>'`.
fn fish_append_vals(s: &mut String, cond: &str, vals: &[(&str, &str)]) {
    for (val, key) in vals {
        s.push_str(&format!(
            "complete -c nvm -n '{}' -a '{}' -d '{}'\n",
            cond,
            val,
            T(key)
        ));
    }
}

fn build_bash_script() -> String {
    r#"# nvm bash completion
_nvm_completion() {
    local cur prev words cword
    # Use _init_completion from bash-completion if available; fall back to
    # manual COMP_WORDS parsing so the script works without bash-completion.
    if declare -f _init_completion >/dev/null 2>&1; then
        _init_completion -n=: || return
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD-1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    fi

    local commands="install uninstall remove use list ls ls-remote remote current dir which run exec alias unalias mirror auto deactivate unload install-npm install-yarn install-pnpm reinstall-packages version version-remote cache language lang proxy completion corepack migrate upgrade update refresh init doctor help"
    local options="--lts --latest --lts-newer --lts-old --offline --source --no-gpg-verify --reinstall-packages-from --latest-npm --latest-yarn --latest-pnpm --install-if-missing --save --use-on-cd --filter --sort --page --check --force --from-gitee --from-mirror --rollback --silent"

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
"#
    .to_string()
}

fn bash_completions() -> anyhow::Result<()> {
    let script = build_bash_script();
    write_completion(
        "nvm.bash",
        &script,
        "completions_written_bash",
        "add_to_bashrc",
        None,
        |f| vec![format!("source {}", f.display())],
    )
}

fn build_zsh_script() -> String {
    // Command descriptions: (command name, i18n key). Aliases (remove/ls/lang)
    // reuse the main command description -- no "(alias)" tag needed, the name
    // itself identifies it.
    let cmds: &[(&str, &str)] = &[
        ("install", "help_install_about"),
        ("uninstall", "help_uninstall_about"),
        ("remove", "help_uninstall_about"),
        ("use", "help_use_about"),
        ("list", "help_list_about"),
        ("ls", "help_list_about"),
        ("ls-remote", "help_remote_about"),
        ("remote", "help_remote_about"),
        ("current", "help_current_about"),
        ("dir", "help_dir_about"),
        ("which", "help_which_about"),
        ("run", "help_run_about"),
        ("exec", "help_exec_about"),
        ("alias", "help_alias_about"),
        ("unalias", "help_unalias_about"),
        ("mirror", "help_mirror_about"),
        ("auto", "help_auto_about"),
        ("deactivate", "help_deactivate_about"),
        ("unload", "help_unload_about"),
        ("install-npm", "help_install_npm_about"),
        ("install-yarn", "help_install_yarn_about"),
        ("install-pnpm", "help_install_pnpm_about"),
        ("reinstall-packages", "help_reinstall_about"),
        ("version", "help_version_about"),
        ("version-remote", "help_version_remote_about"),
        ("cache", "help_cache_about"),
        ("language", "help_language_about"),
        ("lang", "help_language_about"),
        ("proxy", "help_proxy_about"),
        ("completion", "help_completion_about"),
        ("corepack", "help_corepack_about"),
        ("migrate", "help_migrate_about"),
        ("upgrade", "help_upgrade_about"),
        // `update` is a clap alias of `upgrade` (cli.rs), so it is accepted and
        // should be completable too.
        ("update", "help_upgrade_about"),
        ("refresh", "help_refresh_about"),
        ("init", "help_init_about"),
        ("doctor", "help_doctor_about"),
        ("help", "help_root_print_help"),
    ];

    // Option descriptions: (flag, value suffix, i18n key).
    // flag like "--lts" -> '--lts[desc]'; "--sort=" with ":order:(desc asc)" ->
    // '--sort=[desc]:order:(desc asc)'. The value suffix is zsh value-completion
    // syntax and is not translated.
    let install_opts: &[(&str, &str, &str)] = &[
        ("--lts", "", "help_install_lts"),
        ("--latest", "", "help_install_latest"),
        ("--lts-newer", "", "help_install_lts_newer"),
        ("--offline", "", "help_install_offline"),
        ("--source", "", "help_install_source"),
        ("--no-gpg-verify", "", "help_install_no_gpg_verify"),
        ("--latest-npm", "", "help_install_latest_npm"),
        ("--latest-yarn", "", "help_install_latest_yarn"),
        ("--latest-pnpm", "", "help_install_latest_pnpm"),
        (
            "--reinstall-packages-from=",
            ":ver:",
            "help_install_reinstall",
        ),
    ];
    let remote_opts: &[(&str, &str, &str)] = &[
        ("--lts", "", "help_remote_lts"),
        ("--lts-old", "", "help_remote_lts_old"),
        ("--filter=", ":pattern:", "help_remote_filter"),
        ("--sort=", ":order:(desc asc)", "help_remote_sort"),
        ("--page=", ":page:", "help_remote_page_arg"),
    ];
    let use_opts: &[(&str, &str, &str)] = &[
        ("--install-if-missing", "", "help_use_install_if_missing"),
        ("--save", "", "help_use_save"),
        ("--use-on-cd", "", "help_use_use_on_cd"),
    ];
    let uninstall_opts: &[(&str, &str, &str)] = &[
        ("--lts", "", "help_uninstall_lts"),
        ("--latest", "", "help_uninstall_latest"),
    ];
    let upgrade_opts: &[(&str, &str, &str)] = &[
        ("--check", "", "help_upgrade_check"),
        ("--force", "", "help_upgrade_force"),
        ("--from-gitee", "", "help_upgrade_from_gitee"),
        ("--from-mirror=", ":url:", "help_upgrade_from_mirror"),
        ("--rollback", "", "help_upgrade_rollback"),
    ];
    let auto_opts: &[(&str, &str, &str)] = &[("--silent", "", "help_auto_silent")];

    let mut s = String::new();
    s.push_str("#compdef nvm\n\n");

    // _nvm_commands
    s.push_str("_nvm_commands() {\n    local commands\n    commands=(\n");
    for (name, key) in cmds {
        s.push_str(&format!("        '{}:{}'\n", name, T(key)));
    }
    s.push_str("    )\n    _describe 'command' commands\n}\n\n");

    zsh_append_opts(&mut s, "_nvm_install", install_opts);
    zsh_append_opts(&mut s, "_nvm_remote", remote_opts);
    zsh_append_opts(&mut s, "_nvm_use", use_opts);
    zsh_append_opts(&mut s, "_nvm_uninstall", uninstall_opts);
    zsh_append_opts(&mut s, "_nvm_upgrade", upgrade_opts);
    zsh_append_opts(&mut s, "_nvm_auto", auto_opts);

    // _nvm main function: state machine, no description text, kept static
    s.push_str(
        r#"_nvm() {
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
                use)
                    _nvm_use_opts
                    _message 'version'
                    ;;
                uninstall|remove)
                    _nvm_uninstall_opts
                    _message 'version'
                    ;;
                auto)
                    _nvm_auto_opts
                    ;;
                run|exec|which|alias|unalias|reinstall-packages|install-npm|install-yarn|install-pnpm)
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
                upgrade)
                    _nvm_upgrade_opts
                    ;;
            esac
            ;;
    esac
}

"#,
    );

    s
}

fn zsh_completions() -> anyhow::Result<()> {
    let script = build_zsh_script();
    // Detect whether ~/.zshrc has already initialized the completion system.
    // zsh's fpath + autoload only actually load completion functions after
    // compinit is called; if the user's .zshrc lacks compinit, adding
    // fpath/autoload alone has no effect (Tab completion does nothing). When
    // missing, we additionally prompt the user to add
    // `autoload -Uz compinit && compinit`, and prepend that line to
    // install_lines so the user can copy-paste directly.
    let zshrc = Some(std::path::PathBuf::from(crate::system::get_home_dir()).join(".zshrc"));
    let has_compinit = zshrc
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|content| content.contains("compinit"))
        .unwrap_or(false);
    let note = if has_compinit {
        None
    } else {
        Some(T("zsh_needs_compinit").to_string())
    };
    write_completion(
        "_nvm",
        &script,
        "completions_written_zsh",
        "add_to_zshrc",
        note.as_deref(),
        |f| {
            // zsh `fpath` is a list of DIRECTORIES, not files. The completion
            // file _nvm lives in nvm_dir/completions, so that directory is
            // what we add to fpath. Passing the file path itself (e.g.
            // .../completions/_nvm) makes `autoload -Uz _nvm` fail to find
            // the function, silently disabling all zsh completion.
            let dir = f.parent().unwrap_or(f);
            let mut lines = Vec::new();
            if !has_compinit {
                // Prepend before fpath: compinit must initialize the completion
                // system first, only then can the newly added _nvm in fpath be
                // loaded via autoload.
                lines.push("autoload -Uz compinit && compinit".to_string());
            }
            lines.push(format!("fpath=( {} $fpath )", dir.display()));
            lines.push("autoload -Uz _nvm".to_string());
            lines
        },
    )?;
    println!();
    println!(
        "  {} {}",
        crate::i18n::T("tip_label").dimmed(),
        crate::i18n::T("completion_cache_tip").dimmed()
    );
    Ok(())
}

fn build_fish_script() -> String {
    // Command descriptions: (command name, i18n key) -- same as zsh, aliases
    // reuse the main command description.
    let cmds: &[(&str, &str)] = &[
        ("install", "help_install_about"),
        ("uninstall", "help_uninstall_about"),
        ("remove", "help_uninstall_about"),
        ("use", "help_use_about"),
        ("list", "help_list_about"),
        ("ls", "help_list_about"),
        ("ls-remote", "help_remote_about"),
        ("remote", "help_remote_about"),
        ("current", "help_current_about"),
        ("dir", "help_dir_about"),
        ("which", "help_which_about"),
        ("run", "help_run_about"),
        ("exec", "help_exec_about"),
        ("alias", "help_alias_about"),
        ("unalias", "help_unalias_about"),
        ("mirror", "help_mirror_about"),
        ("auto", "help_auto_about"),
        ("deactivate", "help_deactivate_about"),
        ("unload", "help_unload_about"),
        ("install-npm", "help_install_npm_about"),
        ("install-yarn", "help_install_yarn_about"),
        ("install-pnpm", "help_install_pnpm_about"),
        ("reinstall-packages", "help_reinstall_about"),
        ("version", "help_version_about"),
        ("version-remote", "help_version_remote_about"),
        ("cache", "help_cache_about"),
        ("language", "help_language_about"),
        ("lang", "help_language_about"),
        ("proxy", "help_proxy_about"),
        ("completion", "help_completion_about"),
        ("corepack", "help_corepack_about"),
        ("migrate", "help_migrate_about"),
        ("upgrade", "help_upgrade_about"),
        // `update` is a clap alias of `upgrade` (cli.rs), so it is accepted and
        // should be completable too.
        ("update", "help_upgrade_about"),
        ("refresh", "help_refresh_about"),
        ("init", "help_init_about"),
        ("doctor", "help_doctor_about"),
        ("help", "help_root_print_help"),
    ];

    // Options: (fish condition, flag, i18n key)
    let install_opts: &[(&str, &str, &str)] = &[
        (
            "__fish_seen_subcommand_from install",
            "lts",
            "help_install_lts",
        ),
        (
            "__fish_seen_subcommand_from install",
            "latest",
            "help_install_latest",
        ),
        (
            "__fish_seen_subcommand_from install",
            "lts-newer",
            "help_install_lts_newer",
        ),
        (
            "__fish_seen_subcommand_from install",
            "offline",
            "help_install_offline",
        ),
        (
            "__fish_seen_subcommand_from install",
            "source",
            "help_install_source",
        ),
        (
            "__fish_seen_subcommand_from install",
            "no-gpg-verify",
            "help_install_no_gpg_verify",
        ),
        (
            "__fish_seen_subcommand_from install",
            "latest-npm",
            "help_install_latest_npm",
        ),
        (
            "__fish_seen_subcommand_from install",
            "latest-yarn",
            "help_install_latest_yarn",
        ),
        (
            "__fish_seen_subcommand_from install",
            "latest-pnpm",
            "help_install_latest_pnpm",
        ),
        (
            "__fish_seen_subcommand_from install",
            "reinstall-packages-from",
            "help_install_reinstall",
        ),
    ];
    let remote_cond =
        "__fish_seen_subcommand_from remote; or __fish_seen_subcommand_from ls-remote";
    let remote_opts: &[(&str, &str, &str)] = &[
        (remote_cond, "lts", "help_remote_lts"),
        (remote_cond, "lts-old", "help_remote_lts_old"),
        (remote_cond, "filter", "help_remote_filter"),
        (remote_cond, "sort", "help_remote_sort"),
        (remote_cond, "page", "help_remote_page_arg"),
    ];
    let use_opts: &[(&str, &str, &str)] = &[
        (
            "__fish_seen_subcommand_from use",
            "install-if-missing",
            "help_use_install_if_missing",
        ),
        ("__fish_seen_subcommand_from use", "save", "help_use_save"),
        (
            "__fish_seen_subcommand_from use",
            "use-on-cd",
            "help_use_use_on_cd",
        ),
    ];
    let upgrade_opts: &[(&str, &str, &str)] = &[
        (
            "__fish_seen_subcommand_from upgrade",
            "check",
            "help_upgrade_check",
        ),
        (
            "__fish_seen_subcommand_from upgrade",
            "force",
            "help_upgrade_force",
        ),
        (
            "__fish_seen_subcommand_from upgrade",
            "from-gitee",
            "help_upgrade_from_gitee",
        ),
        (
            "__fish_seen_subcommand_from upgrade",
            "from-mirror",
            "help_upgrade_from_mirror",
        ),
        (
            "__fish_seen_subcommand_from upgrade",
            "rollback",
            "help_upgrade_rollback",
        ),
    ];
    let auto_opts: &[(&str, &str, &str)] = &[(
        "__fish_seen_subcommand_from auto",
        "silent",
        "help_auto_silent",
    )];

    // Value completions: (value, i18n key)
    let mirror_vals: &[(&str, &str)] = &[
        ("taobao", "comp_mirror_taobao"),
        ("official", "comp_mirror_official"),
        ("npmmirror", "comp_mirror_npmmirror"),
    ];
    let lang_cond = "__fish_seen_subcommand_from language; or __fish_seen_subcommand_from lang";
    let lang_vals: &[(&str, &str)] = &[("en", "comp_lang_en"), ("cn", "comp_lang_cn")];
    let proxy_vals: &[(&str, &str)] = &[("on", "comp_proxy_on"), ("off", "comp_proxy_off")];
    let corepack_vals: &[(&str, &str)] = &[
        ("enable", "comp_corepack_enable"),
        ("disable", "comp_corepack_disable"),
        ("status", "comp_corepack_status"),
    ];
    let cache_vals: &[(&str, &str)] = &[
        ("dir", "help_cache_dir"),
        ("list", "help_cache_list"),
        ("clear", "help_cache_clear"),
    ];
    let migrate_vals: &[(&str, &str)] = &[
        ("nvm", "comp_migrate_nvm"),
        ("nvm-windows", "comp_migrate_nvm_windows"),
    ];

    let mut s = String::new();
    s.push_str("# nvm fish completion\n\n");

    // Command completions
    for (name, key) in cmds {
        s.push_str(&format!(
            "complete -c nvm -n '__fish_use_subcommand' -a '{}' -d '{}'\n",
            name,
            T(key)
        ));
    }
    s.push('\n');

    // Option completions
    fish_append_opts(&mut s, upgrade_opts);
    fish_append_opts(&mut s, auto_opts);
    fish_append_opts(&mut s, install_opts);
    fish_append_opts(&mut s, use_opts);
    fish_append_opts(&mut s, remote_opts);
    s.push('\n');

    // Value completions
    fish_append_vals(&mut s, "__fish_seen_subcommand_from mirror", mirror_vals);
    fish_append_vals(&mut s, lang_cond, lang_vals);
    fish_append_vals(&mut s, "__fish_seen_subcommand_from proxy", proxy_vals);
    fish_append_vals(
        &mut s,
        "__fish_seen_subcommand_from corepack",
        corepack_vals,
    );
    fish_append_vals(&mut s, "__fish_seen_subcommand_from cache", cache_vals);
    fish_append_vals(&mut s, "__fish_seen_subcommand_from migrate", migrate_vals);

    // shell values: Bash/Zsh/Fish/PowerShell are proper nouns, not translated
    for (val, desc) in &[
        ("bash", "Bash"),
        ("zsh", "Zsh"),
        ("fish", "Fish"),
        ("powershell", "PowerShell"),
    ] {
        s.push_str(&format!(
            "complete -c nvm -n '__fish_seen_subcommand_from completion' -a '{}' -d '{}'\n",
            val, desc
        ));
    }

    s
}

fn fish_completions() -> anyhow::Result<()> {
    let script = build_fish_script();
    write_completion(
        "nvm.fish",
        &script,
        "completions_written_fish",
        "add_to_fish_config",
        None,
        |f| vec![format!("source {}", f.display())],
    )
}

fn build_powershell_script() -> String {
    r#"# nvm PowerShell completion

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
    'upgrade',
    'update',
    'refresh',
    'init',
    'doctor',
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
    '--page',
    '--check',
    '--force',
    '--from-gitee',
    '--from-mirror',
    '--rollback',
    '--silent'
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
"#
    .to_string()
}

fn powershell_completions() -> anyhow::Result<()> {
    let script = build_powershell_script();
    write_completion(
        "nvm.ps1",
        &script,
        "completions_written_powershell",
        "add_to_powershell_profile",
        None,
        |f| vec![format!(". {}", f.display())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::KNOWN_COMMANDS;

    /// Point `NVM_DIR` at an empty temp dir for the duration of a test.
    ///
    /// `build_zsh_script` and `build_fish_script` interpolate `T(key)`, which
    /// resolves the language from `NVM_DIR/config.json`; without this the tests
    /// would read the developer's real `~/.nvm.rust/`. Note `get_language`
    /// caches process-wide, so the resolved language is not deterministic here
    /// — which is fine, because every assertion below is on command names and
    /// shell keywords, never on translated description text.
    struct NvmDirGuard {
        old: Option<std::ffi::OsString>,
        _dir: tempfile::TempDir,
        _mutex: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for NvmDirGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var("NVM_DIR", v),
                None => std::env::remove_var("NVM_DIR"),
            }
        }
    }

    fn isolate_nvm_dir() -> NvmDirGuard {
        let mutex = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old = std::env::var_os("NVM_DIR");
        let dir = tempfile::TempDir::new().expect("tempdir for NVM_DIR");
        std::env::set_var("NVM_DIR", dir.path());
        NvmDirGuard {
            old,
            _dir: dir,
            _mutex: mutex,
        }
    }

    /// The bash `local commands="a b c"` word list.
    fn bash_commands(script: &str) -> Vec<String> {
        const MARKER: &str = "local commands=\"";
        let start = script
            .find(MARKER)
            .expect("bash completion must declare a commands list")
            + MARKER.len();
        let rest = &script[start..];
        let end = rest.find('"').expect("bash commands list must be closed");
        rest[..end].split_whitespace().map(str::to_string).collect()
    }

    /// Names from the zsh `commands=( 'name:description' ... )` array.
    fn zsh_commands(script: &str) -> Vec<String> {
        const MARKER: &str = "commands=(";
        let start = script
            .find(MARKER)
            .expect("zsh completion must build a commands array")
            + MARKER.len();
        let rest = &script[start..];
        let end = rest.find("\n    )").expect("zsh commands array must close");
        rest[..end]
            .lines()
            .filter_map(|l| {
                let inner = l.trim().strip_prefix('\'')?.strip_suffix('\'')?;
                Some(inner.split(':').next()?.to_string())
            })
            .collect()
    }

    /// Names from the fish `complete -c nvm -n '__fish_use_subcommand' -a 'name'` lines.
    fn fish_commands(script: &str) -> Vec<String> {
        const MARKER: &str = "-n '__fish_use_subcommand' -a '";
        script
            .lines()
            .filter_map(|l| {
                let start = l.find(MARKER)? + MARKER.len();
                let rest = &l[start..];
                Some(rest[..rest.find('\'')?].to_string())
            })
            .collect()
    }

    /// Names from the PowerShell `$commands = @( 'a', 'b' )` array.
    fn powershell_commands(script: &str) -> Vec<String> {
        const MARKER: &str = "$commands = @(";
        let start = script
            .find(MARKER)
            .expect("powershell completion must declare $commands")
            + MARKER.len();
        let rest = &script[start..];
        let end = rest.find(')').expect("powershell $commands must close");
        rest[..end]
            .lines()
            .map(|l| {
                l.trim()
                    .trim_end_matches(',')
                    .trim_matches('\'')
                    .to_string()
            })
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Both directions: nothing the CLI accepts may be uncompletable, and
    /// nothing uncompletable-but-offered may be a command the CLI rejects.
    ///
    /// `help` is exempt from the second check — it is not a clap subcommand but
    /// is intercepted by `cli::intercept_help`, so offering it is correct.
    fn assert_command_set_matches(shell: &str, offered: Vec<String>) {
        assert!(
            offered.len() >= KNOWN_COMMANDS.len(),
            "{shell} completion offered only {} commands, fewer than the {} the CLI accepts — \
             the extractor probably broke",
            offered.len(),
            KNOWN_COMMANDS.len()
        );

        let mut missing: Vec<&str> = KNOWN_COMMANDS
            .iter()
            .filter(|c| !offered.iter().any(|o| o == **c))
            .copied()
            .collect();
        missing.sort_unstable();
        assert!(
            missing.is_empty(),
            "{shell} completion cannot tab-complete these accepted commands: {missing:?}"
        );

        let mut extra: Vec<&str> = offered
            .iter()
            .filter(|o| *o != "help" && !KNOWN_COMMANDS.contains(&o.as_str()))
            .map(String::as_str)
            .collect();
        extra.sort_unstable();
        assert!(
            extra.is_empty(),
            "{shell} completion offers commands the CLI does not accept: {extra:?}"
        );
    }

    /// The four per-shell command lists are maintained separately (zsh and fish
    /// carry a description per command), so they drift. They did: `refresh`,
    /// `init`, `doctor` and the `update` alias were all missing, so users could
    /// not tab-complete them. `cli::KNOWN_COMMANDS` is the authoritative set.
    #[test]
    fn every_shell_completion_matches_the_cli_command_set() {
        let _guard = isolate_nvm_dir();
        assert_command_set_matches("bash", bash_commands(&build_bash_script()));
        assert_command_set_matches("zsh", zsh_commands(&build_zsh_script()));
        assert_command_set_matches("fish", fish_commands(&build_fish_script()));
        assert_command_set_matches(
            "powershell",
            powershell_commands(&build_powershell_script()),
        );
    }

    #[test]
    fn bash_completion_is_structurally_valid() {
        let s = build_bash_script();
        for marker in [
            "_nvm_completion()",
            // bash-completion integration plus the manual COMP_WORDS fallback,
            // so the script works either way.
            "_init_completion",
            "COMP_WORDS",
            "compgen -W \"$commands\"",
            "complete -F _nvm_completion nvm",
        ] {
            assert!(s.contains(marker), "bash completion missing {marker:?}");
        }
        // Flags with no other discoverable surface.
        for flag in [
            "--save",
            "--silent",
            "--lts",
            "--install-if-missing",
            "--reinstall-packages-from",
        ] {
            assert!(s.contains(flag), "bash completion missing flag {flag}");
        }
    }

    #[test]
    fn zsh_completion_is_structurally_valid() {
        let _guard = isolate_nvm_dir();
        let s = build_zsh_script();
        assert!(
            s.starts_with("#compdef nvm"),
            "zsh completion must begin with the #compdef tag or zsh will not load it; \
             starts with {:?}",
            &s[..s.len().min(48)]
        );
        for marker in [
            "_nvm_commands()",
            "_describe 'command' commands",
            "_arguments",
        ] {
            assert!(s.contains(marker), "zsh completion missing {marker:?}");
        }
    }

    #[test]
    fn fish_completion_is_structurally_valid() {
        let _guard = isolate_nvm_dir();
        let s = build_fish_script();
        assert!(
            s.contains("__fish_use_subcommand"),
            "fish completion missing the subcommand guard"
        );
        // Every line must be a `complete` invocation or a comment; anything
        // else is a syntax error the moment the file is sourced.
        for line in s.lines() {
            let t = line.trim();
            assert!(
                t.is_empty() || t.starts_with('#') || t.starts_with("complete -c nvm"),
                "unexpected line in fish completion: {line:?}"
            );
        }
    }

    #[test]
    fn powershell_completion_is_structurally_valid() {
        let s = build_powershell_script();
        for marker in [
            "Register-ArgumentCompleter",
            "-CommandName nvm",
            "-Native",
            "[System.Management.Automation.CompletionResult]",
        ] {
            assert!(
                s.contains(marker),
                "powershell completion missing {marker:?}"
            );
        }
    }
}
