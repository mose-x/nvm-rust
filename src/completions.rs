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

/// 切换语言后静默重新生成已安装的全部 shell 补全脚本。
///
/// zsh/fish 含描述文本，跟随语言重新生成；bash/powershell 虽无描述文本，
/// 也一并重写以保持 4 种补全内容一致（如未来加入命令/选项也能同步更新）。
/// 只覆盖已存在的文件——用户未安装补全时不创建任何文件，避免凭空生成。
/// 静默执行：不打印 "已写入" / "添加到 rc" 等提示，避免污染 `nvm language` 输出。
/// 重新生成后用当前语言的 T() 描述覆盖旧文件，下次打开新 shell 即生效。
pub fn regenerate_completions_if_installed() -> anyhow::Result<()> {
    let completions_dir = get_nvm_dir().join("completions");
    // 只覆盖已存在的文件——用户未安装补全时不创建任何文件。
    // bash/powershell 虽无描述文本，也一并重写以保持 4 种补全内容一致
    //（如未来加入命令/选项也能同步更新）。
    let bash = completions_dir.join("nvm.bash");
    if bash.exists() {
        fs::write(&bash, build_bash_script())?;
    }
    let zsh = completions_dir.join("_nvm");
    if zsh.exists() {
        fs::write(&zsh, build_zsh_script())?;
    }
    let fish = completions_dir.join("nvm.fish");
    if fish.exists() {
        fs::write(&fish, build_fish_script())?;
    }
    let ps1 = completions_dir.join("nvm.ps1");
    if ps1.exists() {
        fs::write(&ps1, build_powershell_script())?;
    }
    Ok(())
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
    _init_completion -n=: || return

    local commands="install uninstall remove use list ls ls-remote remote current dir which run exec alias unalias mirror auto deactivate unload install-npm install-yarn install-pnpm reinstall-packages version version-remote cache language lang proxy completion corepack migrate upgrade help"
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
    // 命令描述: (命令名, i18n key)。别名 (remove/ls/lang) 复用主命令描述——
    // 补全描述无需标注 "(alias)"，命令名本身已表明身份。
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
        ("help", "help_root_print_help"),
    ];

    // 选项描述: (flag, 值后缀, i18n key)。
    // flag 如 "--lts" → '--lts[desc]'；"--sort=" 配合 ":order:(desc asc)" →
    // '--sort=[desc]:order:(desc asc)'。值后缀是 zsh 值补全语法，不翻译。
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

    // _nvm 主函数: state machine，无描述文本，保持静态
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
    // 检测 ~/.zshrc 是否已初始化补全系统。zsh 的 fpath + autoload 只有在
    // compinit 被调用后才会真正加载补全函数；若用户 .zshrc 没有 compinit，
    // 光加 fpath/autoload 不生效（tab 补全无反应）。检测到缺失时额外提示
    // 用户补一行 `autoload -Uz compinit && compinit`，并在 install_lines
    // 最前面带上这行，方便用户直接复制。
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
                // 放在 fpath 之前：compinit 必须先初始化补全系统，
                // 之后 fpath 里新增的 _nvm 才会被 autoload 加载。
                lines.push("autoload -Uz compinit && compinit".to_string());
            }
            lines.push(format!("fpath=( {} $fpath )", dir.display()));
            lines.push("autoload -Uz _nvm".to_string());
            lines
        },
    )
}

fn build_fish_script() -> String {
    // 命令描述: (命令名, i18n key) — 同 zsh，别名复用主命令描述。
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
        ("help", "help_root_print_help"),
    ];

    // 选项: (fish 条件, flag, i18n key)
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

    // 值补全: (值, i18n key)
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

    // 命令补全
    for (name, key) in cmds {
        s.push_str(&format!(
            "complete -c nvm -n '__fish_use_subcommand' -a '{}' -d '{}'\n",
            name,
            T(key)
        ));
    }
    s.push('\n');

    // 选项补全
    fish_append_opts(&mut s, upgrade_opts);
    fish_append_opts(&mut s, auto_opts);
    fish_append_opts(&mut s, install_opts);
    fish_append_opts(&mut s, use_opts);
    fish_append_opts(&mut s, remote_opts);
    s.push('\n');

    // 值补全
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

    // shell 值: Bash/Zsh/Fish/PowerShell 是专有名词，不翻译
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
