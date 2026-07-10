use clap::{Parser, Subcommand};

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
    "install", "use", "list", "ls", "remote", "ls-remote", "uninstall", "remove",
    "current", "dir", "alias", "unalias", "mirror", "run", "exec", "which", "auto",
    "deactivate", "unload", "install-npm", "install-yarn",
    "install-pnpm", "reinstall-packages", "version",
    "version-remote", "cache", "language", "lang", "proxy", "completion", "corepack",
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

    // `nvm <cmd> -h` / `nvm <cmd> --help`
    let cmd = argv[0].as_str();
    if KNOWN_COMMANDS.contains(&cmd) {
        for arg in &argv[1..] {
            if arg == "-h" || arg == "--help" {
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

pub fn print_help() {
    use crate::i18n::T;
    println!("{}", T("help_title"));
    println!();
    println!("{}", T("help_usage_line"));
    println!();
    println!("{}", T("help_core_commands"));
    println!("  nvm install <ver>            {}", T("help_desc_install"));
    println!("  nvm uninstall <ver>          {}", T("help_desc_uninstall"));
    println!("  nvm remove <ver>             {}", T("help_desc_remove"));
    println!("  nvm use <ver>                {}", T("help_desc_use"));
    println!("  nvm list / ls                {}", T("help_desc_list"));
    println!("  nvm remote / ls-remote        {}", T("help_desc_remote"));
    println!("  nvm current                  {}", T("help_desc_current"));
    println!("  nvm dir                      {}", T("help_desc_dir"));
    println!("  nvm which [ver]              {}", T("help_desc_which"));
    println!("  nvm run <ver> [args...]      {}", T("help_desc_run"));
    println!("  nvm exec <ver> <cmd...>      {}", T("help_desc_exec"));
    println!();
    println!("{}", T("help_alias_commands"));
    println!("  nvm alias [name] [ver]        {}", T("help_desc_alias"));
    println!("  nvm unalias <name>           {}", T("help_desc_unalias"));
    println!();
    println!("{}", T("help_env_commands"));
    println!("  nvm auto                     {}", T("help_desc_auto"));
    println!("  nvm deactivate               {}", T("help_desc_deactivate"));
    println!("  nvm unload                   {}", T("help_desc_unload"));
    println!();
    println!("{}", T("help_package_commands"));
    println!("  nvm install-npm [ver]   {}", T("help_desc_install_npm"));
    println!("  nvm install-yarn [ver]  {}", T("help_desc_install_yarn"));
    println!("  nvm install-pnpm [ver]  {}", T("help_desc_install_pnpm"));
    println!("  nvm reinstall-packages <ver>   {}", T("help_desc_reinstall"));
    println!();
    println!("{}", T("help_info_commands"));
    println!("  nvm version                   {}", T("help_desc_version"));
    println!("  nvm version-remote            {}", T("help_desc_version_remote"));
    println!("  nvm mirror [url]              {}", T("help_desc_mirror"));
    println!();
    println!("{}", T("help_env_vars"));
    println!("  NVM_DIR                       {}", T("help_desc_env_nvm_dir"));
    println!();
    println!("{}", T("help_special_aliases"));
    println!("  node, stable, unstable        {}", T("help_desc_special_node"));
    println!("  lts, lts/<codename>          {}", T("help_desc_lts_codename"));
    println!("  system                        {}", T("help_desc_system"));
    println!("  default                       {}", T("help_desc_default"));
}

/// Root help for `nvm -h` / `nvm --help` (mirrors clap layout but i18n-aware).
pub fn print_root_help() {
    use crate::i18n::T;
    println!("{}", T("help_title"));
    println!();
    println!("{}", T("help_root_usage"));
    println!();
    println!("{}", T("help_root_commands"));
    println!("  install             {}", T("help_install_about"));
    println!("  use                 {}", T("help_use_about"));
    println!("  list                {}", T("help_list_about"));
    println!("  remote              {}", T("help_remote_about"));
    println!("  uninstall           {}", T("help_uninstall_about"));
    println!("  current             {}", T("help_current_about"));
    println!("  dir                 {}", T("help_dir_about"));
    println!("  alias               {}", T("help_alias_about"));
    println!("  unalias             {}", T("help_unalias_about"));
    println!("  mirror              {}", T("help_mirror_about"));
    println!("  run                 {}", T("help_run_about"));
    println!("  exec                {}", T("help_exec_about"));
    println!("  which               {}", T("help_which_about"));
    println!("  auto                {}", T("help_auto_about"));
    println!("  deactivate          {}", T("help_deactivate_about"));
    println!("  unload              {}", T("help_unload_about"));
    println!("  install-npm  {}", T("help_install_npm_about"));
    println!("  install-yarn {}", T("help_install_yarn_about"));
    println!("  install-pnpm {}", T("help_install_pnpm_about"));
    println!("  reinstall-packages  {}", T("help_reinstall_about"));
    println!("  version             {}", T("help_version_about"));
    println!("  version-remote      {}", T("help_version_remote_about"));
    println!("  cache               {}", T("help_cache_about"));
    println!("  language            {}", T("help_language_about"));
    println!("  proxy               {}", T("help_proxy_about"));
    println!("  completion          {}", T("help_completion_about"));
    println!("  corepack            {}", T("help_corepack_about"));
    println!("  migrate             {}", T("help_migrate_about"));
    println!("  help                {}", T("help_root_print_help"));
    println!();
    println!("{}", T("help_options_label"));
    println!("  -h, --help     {}", T("help_help_flag"));
    println!("  -V, --version  {}", T("help_version_flag"));
}

/// Per-subcommand help for `nvm <cmd> -h` (i18n-aware, mirrors clap layout).
pub fn print_command_help(cmd: &str) {
    use crate::i18n::T;
    let opt = T("help_options_label");
    let args = T("help_arguments_label");
    let hf = T("help_help_flag");
    match cmd {
        "install" => {
            println!("{}", T("help_install_about"));
            println!();
            println!("{}", T("help_install_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_install_version_arg"));
            println!();
            println!("{}", opt);
            println!("      --lts                            {}", T("help_install_lts"));
            println!("      --latest                         {}", T("help_install_latest"));
            println!("      --lts-newer                      {}", T("help_install_lts_newer"));
            println!("      --offline                        {}", T("help_install_offline"));
            println!("      --reinstall-packages-from <ver>  {}", T("help_install_reinstall"));
            println!("      --latest-npm                     {}", T("help_install_latest_npm"));
            println!("      --latest-yarn                    {}", T("help_install_latest_yarn"));
            println!("      --latest-pnpm                    {}", T("help_install_latest_pnpm"));
            println!("  -s, --source                         {}", T("help_install_source"));
            println!("      --no-gpg-verify                  {}", T("help_install_no_gpg_verify"));
            println!("  -h, --help                           {}", hf);
        }
        "use" => {
            println!("{}", T("help_use_about"));
            println!();
            println!("{}", T("help_use_usage"));
            println!();
            println!("{}", args);
            println!("  <VERSION>  {}", T("help_use_version_arg"));
            println!();
            println!("{}", opt);
            println!("      --install-if-missing  {}", T("help_use_install_if_missing"));
            println!("      --save                {}", T("help_use_save"));
            println!("      --use-on-cd           {}", T("help_use_use_on_cd"));
            println!("  -h, --help                {}", hf);
        }
        "list" | "ls" => {
            println!("{}", T("help_list_about"));
            println!();
            println!("{}", T("help_list_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "remote" | "ls-remote" => {
            println!("{}", T("help_remote_about"));
            println!();
            println!("{}", T("help_remote_usage"));
            println!();
            println!("{}", args);
            println!("  [PAGE]  {}", T("help_remote_page_arg"));
            println!();
            println!("{}", opt);
            println!("      --lts               {}", T("help_remote_lts"));
            println!("      --lts-old           {}", T("help_remote_lts_old"));
            println!("      --filter <pattern>  {}", T("help_remote_filter"));
            println!("      --sort <order>      {}", T("help_remote_sort"));
            println!("  -h, --help              {}", hf);
        }
        "uninstall" | "remove" => {
            println!("{}", T("help_uninstall_about"));
            println!();
            println!("{}", T("help_uninstall_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_uninstall_version_arg"));
            println!();
            println!("{}", opt);
            println!("      --lts   {}", T("help_uninstall_lts"));
            println!("  -h, --help  {}", hf);
        }
        "current" => {
            println!("{}", T("help_current_about"));
            println!();
            println!("{}", T("help_current_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "dir" => {
            println!("{}", T("help_dir_about"));
            println!();
            println!("{}", T("help_dir_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "alias" => {
            println!("{}", T("help_alias_about"));
            println!();
            println!("{}", T("help_alias_usage"));
            println!();
            println!("{}", args);
            println!("  [NAME]     {}", T("help_alias_name_arg"));
            println!("  [VERSION]  {}", T("help_alias_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "unalias" => {
            println!("{}", T("help_unalias_about"));
            println!();
            println!("{}", T("help_unalias_usage"));
            println!();
            println!("{}", args);
            println!("  <NAME>  {}", T("help_unalias_name_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "mirror" => {
            println!("{}", T("help_mirror_about"));
            println!();
            println!("{}", T("help_mirror_usage"));
            println!();
            println!("{}", args);
            println!("  [MIRROR]  {}", T("help_mirror_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "run" => {
            println!("{}", T("help_run_about"));
            println!();
            println!("{}", T("help_run_usage"));
            println!();
            println!("{}", args);
            println!("  <VERSION>  {}", T("help_run_version_arg"));
            println!("  [ARGS]...  {}", T("help_run_args_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "exec" => {
            println!("{}", T("help_exec_about"));
            println!();
            println!("{}", T("help_exec_usage"));
            println!();
            println!("{}", args);
            println!("  <VERSION>  {}", T("help_exec_version_arg"));
            println!("  [ARGS]...  {}", T("help_exec_args_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "which" => {
            println!("{}", T("help_which_about"));
            println!();
            println!("{}", T("help_which_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_which_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "auto" => {
            println!("{}", T("help_auto_about"));
            println!();
            println!("{}", T("help_auto_usage"));
            println!();
            println!("{}", opt);
            println!("      --silent  {}", T("help_auto_silent"));
            println!("  -h, --help    {}", hf);
        }
        "deactivate" => {
            println!("{}", T("help_deactivate_about"));
            println!();
            println!("{}", T("help_deactivate_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "unload" => {
            println!("{}", T("help_unload_about"));
            println!();
            println!("{}", T("help_unload_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "install-npm" => {
            println!("{}", T("help_install_npm_about"));
            println!();
            println!("{}", T("help_install_npm_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_install_npm_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "install-yarn" => {
            println!("{}", T("help_install_yarn_about"));
            println!();
            println!("{}", T("help_install_yarn_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_install_yarn_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "install-pnpm" => {
            println!("{}", T("help_install_pnpm_about"));
            println!();
            println!("{}", T("help_install_pnpm_usage"));
            println!();
            println!("{}", args);
            println!("  [VERSION]  {}", T("help_install_pnpm_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "reinstall-packages" => {
            println!("{}", T("help_reinstall_about"));
            println!();
            println!("{}", T("help_reinstall_usage"));
            println!();
            println!("{}", args);
            println!("  <FROM>  {}", T("help_reinstall_from_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "version" => {
            println!("{}", T("help_version_about"));
            println!();
            println!("{}", T("help_version_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "version-remote" => {
            println!("{}", T("help_version_remote_about"));
            println!();
            println!("{}", T("help_version_remote_usage"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "cache" => {
            println!("{}", T("help_cache_about"));
            println!();
            println!("{}", T("help_cache_usage"));
            println!();
            println!("{}", T("help_cache_commands"));
            println!("  dir    {}", T("help_cache_dir"));
            println!("  list   {}", T("help_cache_list"));
            println!("  clear  {}", T("help_cache_clear"));
            println!("  help   {}", T("help_root_print_help"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "language" | "lang" => {
            println!("{}", T("help_language_about"));
            println!();
            println!("{}", T("help_language_usage"));
            println!();
            println!("{}", args);
            println!("  [LANG]  {}", T("help_language_lang_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "proxy" => {
            println!("{}", T("help_proxy_about"));
            println!();
            println!("{}", T("help_proxy_usage"));
            println!();
            println!("{}", args);
            println!("  [ACTION]  {}", T("help_proxy_action_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "completion" => {
            println!("{}", T("help_completion_about"));
            println!();
            println!("{}", T("help_completion_usage"));
            println!();
            println!("{}", args);
            println!("  [shell]  {}", T("help_completion_shell_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "corepack" => {
            println!("{}", T("help_corepack_about"));
            println!();
            println!("{}", T("help_corepack_usage"));
            println!();
            println!("{}", args);
            println!("  [ACTION]   {}", T("help_corepack_action_arg"));
            println!("  [VERSION]  {}", T("help_corepack_version_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        "migrate" => {
            println!("{}", T("help_migrate_about"));
            println!();
            println!("{}", T("help_migrate_usage"));
            println!();
            println!("{}", args);
            println!("  [SOURCE]  {}", T("help_migrate_source_arg"));
            println!();
            println!("{}", opt);
            println!("  -h, --help  {}", hf);
        }
        _ => {
            // Unknown command: fall back to root help
            print_root_help();
        }
    }
}
