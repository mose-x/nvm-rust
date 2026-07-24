use clap::{Parser, Subcommand};

use crate::i18n::T;
use crate::utils::{display_width, pad_right};

/// Render one per-command help block. Centralises the
/// `about / usage / args / options / -h` template that was previously
/// hand-written (with manual spacing) in every arm of `print_command_help`.
///
/// `args` and `options` are `(syntax, desc_key)` pairs — the syntax column
/// is padded to the widest entry in its section (including the trailing
/// `-h, --help` row in the options section) so descriptions align without
/// per-call magic numbers. `extra_sections` is for non-standard sections
/// like `cache`'s "Commands:" block.
fn render_cmd_help(
    about_key: &str,
    usage_key: &str,
    args: &[(&str, &str)],
    options: &[(&str, &str)],
    extra_sections: &[(&str, &[(&str, &str)])],
) {
    let opt_label = T("help_options_label");
    let args_label = T("help_arguments_label");
    let help_flag = T("help_help_flag");

    println!("{}", T(about_key));
    println!();
    println!("{}", T(usage_key));
    println!();

    let print_rows = |rows: &[(&str, &str)]| {
        let max = rows
            .iter()
            .map(|(s, _)| display_width(s))
            .max()
            .unwrap_or(0);
        for (syntax, desc_key) in rows {
            println!("  {}  {}", pad_right(syntax, max), T(desc_key));
        }
    };

    if !args.is_empty() {
        println!("{}", args_label);
        print_rows(args);
        println!();
    }

    for (title_key, items) in extra_sections {
        println!("{}", T(title_key));
        print_rows(items);
        println!();
    }

    println!("{}", opt_label);
    // Pad option rows and the trailing -h/--help to the same column width
    // so the descriptions line up.
    let opt_width = options
        .iter()
        .map(|(s, _)| display_width(s))
        .chain(std::iter::once(display_width("-h, --help")))
        .max()
        .unwrap_or(0);
    for (syntax, desc_key) in options {
        println!("  {}  {}", pad_right(syntax, opt_width), T(desc_key));
    }
    println!("  {}  {}", pad_right("-h, --help", opt_width), help_flag);
}

#[derive(Parser)]
#[clap(name = "nvm", author, version, about = "Node Version Manager (Rust)", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Option<Commands>,
}

/// What kind of help the user asked for, when we intercept it manually.
pub enum HelpAction {
    Root,
    Command(String),
}

/// Commands we recognize for help interception (incl. aliases).
const KNOWN_COMMANDS: &[&str] = &[
    "install",
    "use",
    "list",
    "ls",
    "remote",
    "ls-remote",
    "uninstall",
    "remove",
    "current",
    "dir",
    "alias",
    "unalias",
    "mirror",
    "run",
    "exec",
    "which",
    "auto",
    "deactivate",
    "unload",
    "install-npm",
    "install-yarn",
    "install-pnpm",
    "reinstall-packages",
    "version",
    "version-remote",
    "cache",
    "language",
    "lang",
    "proxy",
    "completion",
    "corepack",
    "migrate",
];

/// Detect help requests (`-h`, `--help`, or `help` subcommand) so we can render
/// i18n-aware help instead of clap's compile-time (English) help.
///
/// Returns `None` when there is no help request (normal dispatch continues).
pub fn intercept_help(argv: &[String]) -> Option<HelpAction> {
    if argv.is_empty() {
        return None;
    }

    // `nvm -h` / `nvm --help`  (no command)
    if argv[0] == "-h" || argv[0] == "--help" {
        return Some(HelpAction::Root);
    }

    // `nvm help`            -> root help
    // `nvm help <cmd>`      -> <cmd> help
    if argv[0] == "help" {
        if argv.len() == 1 {
            return Some(HelpAction::Root);
        }
        let name = argv[1].as_str();
        if KNOWN_COMMANDS.contains(&name) {
            return Some(HelpAction::Command(name.to_string()));
        }
        // `nvm help <unknown>` -> let clap handle (produces an error)
        return None;
    }

    // `nvm <cmd> -h` / `nvm <cmd> --help`. Only inspect the token immediately
    // following <cmd>, NOT the entire argv. Scanning all of argv caused
    // `nvm run 20 --help script.js` (where `--help` is a trailing arg meant
    // for the user's script) to be mis-intercepted as "show run help" and
    // exit, so the script never ran. Restricting to argv[1] matches the
    // common `nvm <cmd> --help` form while leaving trailing-var-arg commands
    // (`run`/`exec`) free to forward `--help` to the child process.
    let cmd = argv[0].as_str();
    if KNOWN_COMMANDS.contains(&cmd) {
        if let Some(first) = argv.get(1) {
            if first == "-h" || first == "--help" {
                return Some(HelpAction::Command(cmd.to_string()));
            }
        }
    }

    None
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Install a specific Node.js version
    Install {
        /// Version number (supports 20, v20.11.0, lts/*, node, stable, etc.)
        version: Option<String>,
        /// Install the latest LTS version
        #[clap(long)]
        lts: bool,
        /// Install the latest release
        #[clap(long)]
        latest: bool,
        /// Install latest LTS only if not already installed
        #[clap(long)]
        lts_newer: bool,
        /// Install from cache only (no network)
        #[clap(long)]
        offline: bool,
        /// Reinstall global packages from a version after install
        #[clap(long, value_name = "ver")]
        reinstall_packages_from: Option<String>,
        /// Upgrade npm to latest after install
        #[clap(long)]
        latest_npm: bool,
        /// Install the latest yarn after install
        #[clap(long)]
        latest_yarn: bool,
        /// Install the latest pnpm after install
        #[clap(long)]
        latest_pnpm: bool,
        /// Compile and install from source (requires compiler toolchain)
        #[clap(long, short)]
        source: bool,
        /// Skip GPG signature verification of SHASUMS256.txt
        #[clap(long)]
        no_gpg_verify: bool,
    },
    /// Switch to a specific Node.js version
    Use {
        /// Version number or alias. If omitted, reads `.nvmrc` /
        /// `.node-version` / `package.json#engines.node` from the current
        /// directory (matching nvm-sh's `nvm use` behavior).
        version: Option<String>,
        /// Install the version if it is not installed yet
        #[clap(long)]
        install_if_missing: bool,
        /// Persist this version as the default (writes to config.json)
        #[clap(long)]
        save: bool,
        /// Enable auto-switch on directory change (installs shell cd hook)
        #[clap(long)]
        use_on_cd: bool,
    },
    /// List locally installed versions
    #[clap(alias = "ls")]
    List,
    /// List remotely available versions
    #[clap(alias = "ls-remote")]
    Remote {
        /// Only show LTS versions
        #[clap(long)]
        lts: bool,
        /// Only show LTS versions <= 18
        #[clap(long)]
        lts_old: bool,
        /// Filter versions (e.g., "20", "18", "16")
        #[clap(long, value_name = "pattern")]
        filter: Option<String>,
        /// Sort order: "desc" (default) or "asc"
        #[clap(long, value_name = "order")]
        sort: Option<String>,
        /// Page number (1-based, 20 per page)
        page: Option<usize>,
    },
    /// Uninstall a specific version
    #[clap(alias = "remove")]
    Uninstall {
        /// Version number (or --lts / --latest to uninstall latest LTS/latest)
        version: Option<String>,
        /// Uninstall the latest LTS version
        #[clap(long)]
        lts: bool,
        /// Uninstall the latest installed version
        #[clap(long)]
        latest: bool,
    },
    /// Show the currently active version
    Current,
    /// Show NVM installation and .nvm.rust paths
    Dir,
    /// Set or show aliases
    Alias {
        /// Alias name (shows all aliases if omitted)
        name: Option<String>,
        /// Version number
        version: Option<String>,
    },
    /// Remove an alias
    Unalias {
        /// Alias name
        name: String,
    },
    /// Set or show the download mirror
    Mirror {
        /// Mirror URL (taobao/official/custom URL)
        mirror: Option<String>,
    },
    /// Run a script with a specific Node.js version
    Run {
        /// Version number
        version: String,
        /// Script or command to run
        #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Execute a command with a specific Node.js version
    Exec {
        /// Version number
        version: String,
        /// Command and arguments
        #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Show the installation path of a version
    Which {
        /// Version number (defaults to current)
        version: Option<String>,
    },
    /// Auto-switch based on .nvmrc or .node-version
    Auto {
        /// Suppress output (used by cd hook)
        #[clap(long)]
        silent: bool,
    },
    /// Deactivate current version (restore PATH)
    Deactivate,
    /// Remove NVM from shell config
    Unload,
    /// Upgrade npm to the latest for a version
    InstallNpm {
        /// Version number (defaults to current)
        version: Option<String>,
    },
    /// Install the latest yarn for a version
    InstallYarn {
        /// Version number (defaults to current)
        version: Option<String>,
    },
    /// Install the latest pnpm for a version
    InstallPnpm {
        /// Version number (defaults to current)
        version: Option<String>,
    },
    /// Migrate global packages from one version to current
    ReinstallPackages {
        /// Source version
        from: String,
    },
    /// Show current node/npm version info
    Version,
    /// Show remote node/npm version info
    VersionRemote,
    /// Cache management
    Cache {
        #[clap(subcommand)]
        action: CacheAction,
    },
    /// Set or show language
    #[clap(alias = "lang")]
    Language {
        /// Language code (en or cn)
        lang: Option<String>,
    },
    /// Manage proxy settings
    Proxy {
        /// Action: on / off (show status if omitted)
        action: Option<String>,
    },
    /// Generate shell completions
    Completion {
        /// Shell type (bash, zsh, fish, powershell)
        #[clap(value_name = "shell")]
        shell: Option<String>,
    },
    /// Enable/disable corepack for a version
    Corepack {
        /// Action: enable / disable / status
        action: Option<String>,
        /// Version to enable/disable corepack for
        version: Option<String>,
    },
    /// Migrate installed versions from nvm-sh or nvm-windows
    Migrate {
        /// Source: nvm (nvm-sh) or nvm-windows
        #[clap(default_value = "nvm")]
        source: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum CacheAction {
    /// Show the cache directory
    Dir,
    /// List cached files
    List,
    /// Clear all cached files
    Clear,
}

/// Print a titled section of `  <cmd>  <desc>` rows, padding the command
/// column to the widest entry so descriptions align without magic numbers.
fn print_cmd_section(title_key: &str, rows: &[(&str, &str)]) {
    println!("{}", T(title_key));
    let max = rows
        .iter()
        .map(|(s, _)| display_width(s))
        .max()
        .unwrap_or(0);
    for (cmd, desc_key) in rows {
        println!("  {}  {}", pad_right(cmd, max), T(desc_key));
    }
}

pub fn print_help() {
    println!("{}", T("help_title"));
    println!();
    println!("{}", T("help_usage_line"));
    println!();
    print_cmd_section(
        "help_core_commands",
        &[
            ("nvm install <ver>", "help_desc_install"),
            ("nvm uninstall <ver>", "help_desc_uninstall"),
            ("nvm remove <ver>", "help_desc_remove"),
            ("nvm use <ver>", "help_desc_use"),
            ("nvm list / ls", "help_desc_list"),
            ("nvm remote / ls-remote", "help_desc_remote"),
            ("nvm current", "help_desc_current"),
            ("nvm dir", "help_desc_dir"),
            ("nvm which [ver]", "help_desc_which"),
            ("nvm run <ver> [args...]", "help_desc_run"),
            ("nvm exec <ver> <cmd...>", "help_desc_exec"),
        ],
    );
    println!();
    print_cmd_section(
        "help_alias_commands",
        &[
            ("nvm alias [name] [ver]", "help_desc_alias"),
            ("nvm unalias <name>", "help_desc_unalias"),
        ],
    );
    println!();
    print_cmd_section(
        "help_env_commands",
        &[
            ("nvm auto", "help_desc_auto"),
            ("nvm deactivate", "help_desc_deactivate"),
            ("nvm unload", "help_desc_unload"),
        ],
    );
    println!();
    print_cmd_section(
        "help_package_commands",
        &[
            ("nvm install-npm [ver]", "help_desc_install_npm"),
            ("nvm install-yarn [ver]", "help_desc_install_yarn"),
            ("nvm install-pnpm [ver]", "help_desc_install_pnpm"),
            ("nvm reinstall-packages <ver>", "help_desc_reinstall"),
        ],
    );
    println!();
    print_cmd_section(
        "help_info_commands",
        &[
            ("nvm version", "help_desc_version"),
            ("nvm version-remote", "help_desc_version_remote"),
            ("nvm mirror [url]", "help_desc_mirror"),
        ],
    );
    println!();
    print_cmd_section("help_env_vars", &[("NVM_DIR", "help_desc_env_nvm_dir")]);
    println!();
    print_cmd_section(
        "help_special_aliases",
        &[
            ("node, stable, unstable", "help_desc_special_node"),
            ("lts, lts/<codename>", "help_desc_lts_codename"),
            ("system", "help_desc_system"),
            ("default", "help_desc_default"),
        ],
    );
}

/// Root help for `nvm -h` / `nvm --help` (mirrors clap layout but i18n-aware).
pub fn print_root_help() {
    println!("{}", T("help_title"));
    println!();
    println!("{}", T("help_root_usage"));
    println!();
    print_cmd_section(
        "help_root_commands",
        &[
            ("install", "help_install_about"),
            ("use", "help_use_about"),
            ("list", "help_list_about"),
            ("remote", "help_remote_about"),
            ("uninstall", "help_uninstall_about"),
            ("current", "help_current_about"),
            ("dir", "help_dir_about"),
            ("alias", "help_alias_about"),
            ("unalias", "help_unalias_about"),
            ("mirror", "help_mirror_about"),
            ("run", "help_run_about"),
            ("exec", "help_exec_about"),
            ("which", "help_which_about"),
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
            ("proxy", "help_proxy_about"),
            ("completion", "help_completion_about"),
            ("corepack", "help_corepack_about"),
            ("migrate", "help_migrate_about"),
            ("help", "help_root_print_help"),
        ],
    );
    println!();
    println!("{}", T("help_options_label"));
    println!("  -h, --help     {}", T("help_help_flag"));
    println!("  -V, --version  {}", T("help_version_flag"));
}

/// Per-subcommand help for `nvm <cmd> -h` (i18n-aware, mirrors clap layout).
pub fn print_command_help(cmd: &str) {
    match cmd {
        "install" => render_cmd_help(
            "help_install_about",
            "help_install_usage",
            &[("[VERSION]", "help_install_version_arg")],
            &[
                ("    --lts", "help_install_lts"),
                ("    --latest", "help_install_latest"),
                ("    --lts-newer", "help_install_lts_newer"),
                ("    --offline", "help_install_offline"),
                (
                    "    --reinstall-packages-from <ver>",
                    "help_install_reinstall",
                ),
                ("    --latest-npm", "help_install_latest_npm"),
                ("    --latest-yarn", "help_install_latest_yarn"),
                ("    --latest-pnpm", "help_install_latest_pnpm"),
                ("-s, --source", "help_install_source"),
                ("    --no-gpg-verify", "help_install_no_gpg_verify"),
            ],
            &[],
        ),
        "use" => render_cmd_help(
            "help_use_about",
            "help_use_usage",
            &[("<VERSION>", "help_use_version_arg")],
            &[
                ("    --install-if-missing", "help_use_install_if_missing"),
                ("    --save", "help_use_save"),
                ("    --use-on-cd", "help_use_use_on_cd"),
            ],
            &[],
        ),
        "list" | "ls" => render_cmd_help("help_list_about", "help_list_usage", &[], &[], &[]),
        "remote" | "ls-remote" => render_cmd_help(
            "help_remote_about",
            "help_remote_usage",
            &[("[PAGE]", "help_remote_page_arg")],
            &[
                ("    --lts", "help_remote_lts"),
                ("    --lts-old", "help_remote_lts_old"),
                ("    --filter <pattern>", "help_remote_filter"),
                ("    --sort <order>", "help_remote_sort"),
            ],
            &[],
        ),
        "uninstall" | "remove" => render_cmd_help(
            "help_uninstall_about",
            "help_uninstall_usage",
            &[("[VERSION]", "help_uninstall_version_arg")],
            &[("    --lts", "help_uninstall_lts")],
            &[],
        ),
        "current" => render_cmd_help("help_current_about", "help_current_usage", &[], &[], &[]),
        "dir" => render_cmd_help("help_dir_about", "help_dir_usage", &[], &[], &[]),
        "alias" => render_cmd_help(
            "help_alias_about",
            "help_alias_usage",
            &[
                ("[NAME]", "help_alias_name_arg"),
                ("[VERSION]", "help_alias_version_arg"),
            ],
            &[],
            &[],
        ),
        "unalias" => render_cmd_help(
            "help_unalias_about",
            "help_unalias_usage",
            &[("<NAME>", "help_unalias_name_arg")],
            &[],
            &[],
        ),
        "mirror" => render_cmd_help(
            "help_mirror_about",
            "help_mirror_usage",
            &[("[MIRROR]", "help_mirror_arg")],
            &[],
            &[],
        ),
        "run" => render_cmd_help(
            "help_run_about",
            "help_run_usage",
            &[
                ("<VERSION>", "help_run_version_arg"),
                ("[ARGS]...", "help_run_args_arg"),
            ],
            &[],
            &[],
        ),
        "exec" => render_cmd_help(
            "help_exec_about",
            "help_exec_usage",
            &[
                ("<VERSION>", "help_exec_version_arg"),
                ("[ARGS]...", "help_exec_args_arg"),
            ],
            &[],
            &[],
        ),
        "which" => render_cmd_help(
            "help_which_about",
            "help_which_usage",
            &[("[VERSION]", "help_which_version_arg")],
            &[],
            &[],
        ),
        "auto" => render_cmd_help(
            "help_auto_about",
            "help_auto_usage",
            &[],
            &[("    --silent", "help_auto_silent")],
            &[],
        ),
        "deactivate" => render_cmd_help(
            "help_deactivate_about",
            "help_deactivate_usage",
            &[],
            &[],
            &[],
        ),
        "unload" => render_cmd_help("help_unload_about", "help_unload_usage", &[], &[], &[]),
        "install-npm" => render_cmd_help(
            "help_install_npm_about",
            "help_install_npm_usage",
            &[("[VERSION]", "help_install_npm_version_arg")],
            &[],
            &[],
        ),
        "install-yarn" => render_cmd_help(
            "help_install_yarn_about",
            "help_install_yarn_usage",
            &[("[VERSION]", "help_install_yarn_version_arg")],
            &[],
            &[],
        ),
        "install-pnpm" => render_cmd_help(
            "help_install_pnpm_about",
            "help_install_pnpm_usage",
            &[("[VERSION]", "help_install_pnpm_version_arg")],
            &[],
            &[],
        ),
        "reinstall-packages" => render_cmd_help(
            "help_reinstall_about",
            "help_reinstall_usage",
            &[("<FROM>", "help_reinstall_from_arg")],
            &[],
            &[],
        ),
        "version" => render_cmd_help("help_version_about", "help_version_usage", &[], &[], &[]),
        "version-remote" => render_cmd_help(
            "help_version_remote_about",
            "help_version_remote_usage",
            &[],
            &[],
            &[],
        ),
        "cache" => render_cmd_help(
            "help_cache_about",
            "help_cache_usage",
            &[],
            &[],
            &[(
                "help_cache_commands",
                &[
                    ("dir", "help_cache_dir"),
                    ("list", "help_cache_list"),
                    ("clear", "help_cache_clear"),
                    ("help", "help_root_print_help"),
                ],
            )],
        ),
        "language" | "lang" => render_cmd_help(
            "help_language_about",
            "help_language_usage",
            &[("[LANG]", "help_language_lang_arg")],
            &[],
            &[],
        ),
        "proxy" => render_cmd_help(
            "help_proxy_about",
            "help_proxy_usage",
            &[("[ACTION]", "help_proxy_action_arg")],
            &[],
            &[],
        ),
        "completion" => render_cmd_help(
            "help_completion_about",
            "help_completion_usage",
            &[("[shell]", "help_completion_shell_arg")],
            &[],
            &[],
        ),
        "corepack" => render_cmd_help(
            "help_corepack_about",
            "help_corepack_usage",
            &[
                ("[ACTION]", "help_corepack_action_arg"),
                ("[VERSION]", "help_corepack_version_arg"),
            ],
            &[],
            &[],
        ),
        "migrate" => render_cmd_help(
            "help_migrate_about",
            "help_migrate_usage",
            &[("[SOURCE]", "help_migrate_source_arg")],
            &[],
            &[],
        ),
        _ => {
            // Unknown command: fall back to root help
            print_root_help();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{intercept_help, HelpAction};

    // `intercept_help` decides whether to render i18n help instead of letting
    // clap dispatch. Two regressions motivated these cases:
    //   1. Scanning ALL of argv for -h/--help made `nvm run 20 --help app.js`
    //      print `run` help instead of forwarding `--help` to the user's
    //      script. Only the token IMMEDIATELY after <cmd> may trigger help.
    //   2. `nvm help <unknown>` must NOT pretend to know the command — it
    //      returns None so clap produces its usual "unrecognized" error.

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn is_root(v: Option<&HelpAction>) -> bool {
        matches!(v, Some(HelpAction::Root))
    }

    fn is_cmd(v: Option<&HelpAction>, name: &str) -> bool {
        matches!(v, Some(HelpAction::Command(c)) if c == name)
    }

    #[test]
    fn empty_argv_is_not_help() {
        assert!(intercept_help(&argv(&[])).is_none());
    }

    #[test]
    fn bare_help_flags_trigger_root_help() {
        assert!(is_root(intercept_help(&argv(&["-h"])).as_ref()));
        assert!(is_root(intercept_help(&argv(&["--help"])).as_ref()));
    }

    #[test]
    fn help_subcommand_without_arg_is_root() {
        assert!(is_root(intercept_help(&argv(&["help"])).as_ref()));
    }

    #[test]
    fn help_subcommand_with_known_cmd_targets_that_cmd() {
        assert!(is_cmd(
            intercept_help(&argv(&["help", "use"])).as_ref(),
            "use"
        ));
        // alias of `list` should be recognised too
        assert!(is_cmd(
            intercept_help(&argv(&["help", "ls"])).as_ref(),
            "ls"
        ));
    }

    #[test]
    fn help_subcommand_with_unknown_cmd_is_none() {
        assert!(intercept_help(&argv(&["help", "does-not-exist"])).is_none());
    }

    #[test]
    fn immediate_help_flag_after_known_cmd_targets_that_cmd() {
        assert!(is_cmd(
            intercept_help(&argv(&["use", "-h"])).as_ref(),
            "use"
        ));
        assert!(is_cmd(
            intercept_help(&argv(&["install", "--help"])).as_ref(),
            "install"
        ));
    }

    #[test]
    fn trailing_help_flag_is_not_intercepted_for_run() {
        // The bug fix: `nvm run 20 --help script.js` must NOT be treated as
        // a help request. argv[1] is "20", not -h/--help, so the parser
        // returns None and `--help` is forwarded to the user's script.
        assert!(intercept_help(&argv(&["run", "20", "--help", "script.js"])).is_none());
        assert!(intercept_help(&argv(&["exec", "20", "node", "--help"])).is_none());
    }

    #[test]
    fn run_with_only_help_flag_targets_run() {
        // `nvm run --help` (no version) — argv[1] IS --help, so this is a
        // legitimate help request for the `run` command itself.
        assert!(is_cmd(
            intercept_help(&argv(&["run", "--help"])).as_ref(),
            "run"
        ));
    }

    #[test]
    fn known_cmd_without_help_flag_is_none() {
        assert!(intercept_help(&argv(&["use", "20"])).is_none());
        assert!(intercept_help(&argv(&["list"])).is_none());
    }

    #[test]
    fn unknown_cmd_with_help_flag_is_none() {
        // `nvm bogus --help` — not a known command, let clap handle it.
        assert!(intercept_help(&argv(&["bogus", "--help"])).is_none());
    }
}
