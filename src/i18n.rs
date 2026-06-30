use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::load_config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Cn,
}

impl Lang {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "en" | "english" => Some(Lang::En),
            "cn" | "zh" | "zh-cn" | "chinese" | "中文" => Some(Lang::Cn),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Cn => "cn",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Cn => "中文",
        }
    }
}

pub fn get_language() -> Lang {
    load_config()
        .ok()
        .and_then(|c| c.language.clone())
        .and_then(|s| Lang::from_str(&s))
        .unwrap_or(Lang::En)
}

pub fn set_language(lang: Lang) -> anyhow::Result<()> {
    let mut config = load_config()?;
    config.language = Some(lang.as_str().to_string());
    crate::config::save_config(&config)?;
    Ok(())
}

#[allow(non_snake_case)]
#[allow(dead_code)]
pub fn T(key: &str) -> String {
    let lang = get_language();
    t(key, lang)
}

/// Format a translation with parameter substitution
pub fn format_t(key: &str, args: &[String]) -> String {
    let template = T(key);
    substitute_params(&template, args)
}

/// Substitute {0}, {1}, etc. in a string with provided arguments
fn substitute_params(template: &str, args: &[String]) -> String {
    let mut result = template.to_string();
    for (i, arg) in args.iter().enumerate() {
        let placeholder = format!("{{{}}}", i);
        result = result.replace(&placeholder, arg);
    }
    result
}

#[allow(dead_code)]
fn t(key: &str, lang: Lang) -> String {
    match lang {
        Lang::En => en_str(key).to_string(),
        Lang::Cn => cn_str(key).to_string(),
    }
}

#[allow(dead_code)]
fn en_str(key: &str) -> &str {
    EN_STRINGS.get(key).copied().unwrap_or(key)
}

#[allow(dead_code)]
fn cn_str(key: &str) -> &str {
    CN_STRINGS.get(key).copied().unwrap_or_else(|| en_str(key))
}

// ---------------------------------------------------------------------------
// String tables
// ---------------------------------------------------------------------------

lazy_static::lazy_static! {
    static ref EN_STRINGS: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("checking_lts", "Checking latest LTS version...");
        m.insert("checking_latest", "Checking latest version...");
        m.insert("checking_remote", "Checking remote versions...");
        m.insert("no_installed_versions", "No installed versions yet.");
        m.insert("run_get_started", "Run {0} to get started");
        m.insert("installing_node", "Installing Node.js");
        m.insert("compiling_node", "Compiling Node.js from source");
        m.insert("installing_iojs", "Installing io.js");
        m.insert("downloading", "Downloading...");
        m.insert("extracting", "Extracting...");
        m.insert("installed_exclaim", "{0} installed!");
        m.insert("already_installed", "{0} is already installed");
        m.insert("compiled", "{0} compiled from source!");
        m.insert("checksum_verified", "✓ verified");
        m.insert("checksum_skipped", "⚠ skipped");
        m.insert("checksum_offline", "⊘ skipped (offline)");
        m.insert("using_cache", "Offline mode: using cache");
        m.insert("cached_file", "[cache] using cached file");
        m.insert("cached_saved", "[cache] saved to cache");
        m.insert("run_use", "Run: nvm use {0}");
        m.insert("url_label", "URL:");
        m.insert("checksum_label", "Checksum:");
        m.insert("cache_dir_label", "Cache directory:");
        m.insert("cache_empty", "Cache is empty.");
        m.insert("cache_cleared", "Cache cleared, freed {0}");
        m.insert("cache_files", "{0} file(s)");
        m.insert("cache_total", "Total: {0}");
        m.insert("cache_title", "Cached files");
        m.insert("nvm_dir_title", "NVM Installation Directory");
        m.insert("nvm_dir_path", "NVM_DIR:");
        m.insert("nvm_home_title", "NVM Home Path");
        m.insert("nvm_home_path", ".nvm.rust:");
        m.insert("installed_versions_title", "Installed Node.js versions");
        m.insert("installed_all_title", "Installed versions");
        m.insert("installed", "{0} installed");
        m.insert("active", "● active: {0}");
        m.insert("no_active", "○ no active version");
        m.insert("remote_title", "Available Node.js versions");
        m.insert("remote_lts_title", "Available LTS versions");
        m.insert("remote_lts_old_title", "Available older LTS versions (<= 18)");
        m.insert("remote_iojs_title", "Available io.js versions");
        m.insert("page_info", "Page {0} of {1} (showing {2}-{3} of {4})");
        m.insert("prev_page", "← nvm remote {0}");
        m.insert("next_page", "nvm remote {0} →");
        m.insert("version", "Version");
        m.insert("lts_col", "LTS");
        m.insert("codename", "Codename");
        m.insert("type_col", "Type");
        m.insert("file_col", "File");
        m.insert("size_col", "Size");
        m.insert("lts_badge", "✓ LTS");
        m.insert("current", "current");
        m.insert("system", "system");
        m.insert("default", "default");
        m.insert("aliases_title", "Aliases:");
        m.insert("alias_set", "✓ {0} → {1}");
        m.insert("alias_removed", "✓ Alias removed: {0}");
        m.insert("alias_not_found", "Alias '{0}' does not exist");
        m.insert("alias_name_empty", "Alias name cannot be empty");
        m.insert("current_mirror", "Current mirror:");
        m.insert("mirror_url_empty", "Mirror URL cannot be empty");
        m.insert("mirror_set", "✓ Mirror set to: {0}");
        m.insert("mirror_official", "✓ Mirror set to official: {0}");
        m.insert("shell_config_removed", "NVM Rust removed from shell config");
        m.insert("not_installed", "Version {0} is not installed");
        m.insert("cannot_resolve", "Cannot resolve version: {0}");
        m.insert("uninstall_warning", "Warning: Uninstalling the current active version");
        m.insert("uninstalling", "Uninstalling {0}...");
        m.insert("uninstalled", "✓ Uninstalled {0}");
        m.insert("current_version", "▶ Current version: {0}");
        m.insert("which_path", "{0}");
        m.insert("auto_switch", "▶ Auto-switching to {0} (from {1})");
        m.insert("no_nvmrc", "ℹ No .nvmrc or .node-version found");
        m.insert("deactivated", "Deactivated current Node.js version");
        m.insert("upgrading_npm", "Upgrading npm for {0}");
        m.insert("npm_upgraded", "npm upgraded successfully");
        m.insert("npm_upgrade_failed", "npm upgrade failed");
        m.insert("npm_upgrade_retry_npx", "First attempt failed, retrying via npx (downloads a fresh npm@latest to a temp dir)...");
        m.insert("reinstall_packages", "Reinstalling packages from {0}");
        m.insert("reinstall_complete", "✓ Reinstall complete");
        m.insert("migrating_packages", "Migrating packages from {0} → {1}");
        m.insert("migration_failed", "Package migration failed: {0}");
        m.insert("version_info", "Version info");
        m.insert("remote_version_info", "Remote version info");
        m.insert("latest_lts", "Latest LTS");
        m.insert("latest_stable", "Latest stable");
        m.insert("source_configure", "./configure --prefix={0}");
        m.insert("source_make", "make -j{0}");
        m.insert("source_install", "make install");
        m.insert("source_extract", "Extracting source...");
        m.insert("source_npm_fetch", "npm not in source build, fetching prebuilt");
        m.insert("downloading_npm", "Downloading npm tarball...");
        m.insert("iojs_source_unsupported", "io.js source compilation is not supported");
        m.insert("offline_no_cache", "File not found in cache: {0}\nTry without --offline to download from network.");
        m.insert("offline_source_no_cache", "Source tarball not in cache: {0}\nTry without --offline to download.");
        m.insert("lang_set", "✓ Language set to {0}");
        m.insert("lang_current", "▶ Current language: {0}");
        m.insert("lang_usage", "Usage: nvm language <en|cn>");
        m.insert("lang_unknown", "Unknown language: {0} (use 'en' or 'cn')");
        m.insert("node_label", "node:");
        m.insert("npm_label", "npm:");
        m.insert("testing_connectivity", "Testing connectivity...");
        m.insert("no_versions_found", "No versions found.");
        m.insert("proxy_no_system_proxy", "No system proxy detected.");
        m.insert("proxy_set_env_vars", "Please set HTTPS_PROXY / HTTP_PROXY environment variables first");
        m.insert("proxy_test_google_ok", "google ✓");
        m.insert("proxy_test_google_fail", "google ✗");
        m.insert("proxy_status_title", "Proxy status");
        // commands.rs additions
        m.insert("fetching_remote", "Fetching remote versions...");
        m.insert("installing_product", "Installing {0}");
        m.insert("compiling_product", "Compiling {0}");
        m.insert("installed_msg", "{0} installed!");
        m.insert("specify_version_lts_latest", "Specify a version, or use --lts / --latest");
        m.insert("specify_version", "Specify a version, or run from a directory with a .nvmrc / .node-version / package.json");
        m.insert("extract_source_failed", "Failed to extract source tarball");
        m.insert("configure_failed", "configure failed");
        m.insert("make_failed", "make failed");
        m.insert("make_install_failed", "make install failed");
        m.insert("version_no_npm", "Version {0} has no npm");
        m.insert("packages_migrated", "{0} packages migrated");
        m.insert("source_not_installed", "Source version {0} is not installed");
        m.insert("target_not_installed", "Target version {0} is not installed");
        m.insert("cannot_fetch_versions", "Cannot fetch version list");
        m.insert("cannot_determine_lts", "Cannot determine latest LTS version");
        m.insert("cannot_determine_latest", "Cannot determine latest version");
        m.insert("cannot_find_url", "Cannot find download URL for {0}");
        m.insert("no_iojs_match", "No io.js versions found matching {0}");
        m.insert("cannot_resolve_iojs", "Cannot resolve io.js version: {0}");
        m.insert("npm_download_failed", "Failed to download npm from {0}");
        m.insert("uninstalling_label", "Uninstalling");
        m.insert("now_using", "Now using");
        m.insert("system_node", "system Node.js");
        m.insert("version_not_installed_installing", "{0} is not installed, installing...");
        m.insert("install_failed", "Failed to install version {0}");
        m.insert("not_installed_run_install", "Version {0} is not installed. Run 'nvm install {0}' first.");
        m.insert("no_installed_lts", "No installed LTS version found. Install one with 'nvm install --lts'.");
        m.insert("package_failed_code", "failed (exit code {0})");
        m.insert("reinstall_failed_list", "Some packages failed to migrate: {0}");
        m.insert("now_using_node", "Now using Node.js");
        m.insert("now_using_iojs", "Now using io.js");
        m.insert("tip_label", "Tip:");
        m.insert("tip_apply_shell", "source ~/.bashrc (or ~/.zshrc) to apply in current shell");
        m.insert("no_active_use", "No active version, run 'nvm use <version>' to set one");
        m.insert("no_current_version_set", "No current version set");
        m.insert("specify_command", "Specify a command to run");
        m.insert("specify_version_or_lts", "Specify a version or use --lts flag");
        m.insert("found_engines_node", "Found engines.node in package.json:");
        m.insert("no_nvmrc_found", "No .nvmrc, .node-version, or package.json with engines.node found");
        m.insert("found_nvmrc", "Found .nvmrc in:");
        m.insert("found_node_version", "Found .node-version in:");
        m.insert("system_node_label", "System node:");
        m.insert("active_node_label", "Active node:");
        m.insert("no_active_version_set", "No active version set");
        m.insert("latest_remote_versions", "Latest remote versions");
        m.insert("pagination_summary", "Page {0}/{1}    {2} total    {3}-{4} showing");
        m.insert("proxy_test_baidu_ok", "baidu ✓");
        m.insert("proxy_test_baidu_fail", "baidu ✗");
        m.insert("proxy_enabled", "Proxy enabled.");
        m.insert("proxy_will_be_used", "System proxy will be used for downloads.");
        m.insert("neither_reachable", "Neither baidu nor google is reachable.");
        m.insert("check_proxy_settings", "Please check your proxy settings first (HTTPS_PROXY / HTTP_PROXY)");
        m.insert("proxy_disabled", "Proxy disabled. Using direct connection.");
        m.insert("unknown_action", "Unknown action: {0}\nUse 'on', 'off', or omit to show status");
        m.insert("proxy_active", "Proxy is active; downloads use system proxy.");
        m.insert("proxy_on_no_env", "Proxy is ON but no system proxy env var detected.");
        m.insert("proxy_off_direct", "Proxy is OFF; downloads use direct connection.");
        m.insert("usage_label", "Usage:");
        m.insert("proxy_usage_hint", "nvm proxy on   /   nvm proxy off");
        m.insert("not_set", "not set");
        // nvm use --save
        m.insert("default_saved", "Default version set to {0}");
        // nvm use --use-on-cd
        m.insert("use_on_cd_enabled", "Auto-switch on cd enabled (shell cd hook installed)");
        // nvm migrate
        m.insert("migrate_scanning", "Scanning {0} for installed versions...");
        m.insert("migrate_no_versions", "No installed versions found at the source.");
        m.insert("migrate_imported", "Imported {0}");
        m.insert("migrate_skipped", "{0} already exists, skipped");
        m.insert("migrate_failed", "Failed to import {0}");
        m.insert("migrate_source_not_found", "Migration source '{0}' not found. Set NVM_SH_HOME to point at a non-default nvm-sh location.");
        m.insert("migrate_default_set", "Default version set to {0}");
        m.insert("migrate_summary", "Migrated {0} version(s), skipped {1}");
        m.insert("language_set_label", "Language set to:");
        m.insert("current_language_label", "Current language:");
        m.insert("nvm_language_hint", "nvm language <en|cn>");
        // config.rs additions
        m.insert("alias_not_exist_msg", "Alias '{0}' does not exist");
        m.insert("no_aliases", "No aliases defined yet.");
        m.insert("no_default_version", "No default version set");
        m.insert("system_node_not_found", "System node not found");
        m.insert("unknown_lts_alias", "Unknown LTS alias: {0}");
        m.insert("no_matching_version", "No matching installed version found (prefix={0})");
        m.insert("no_unstable", "No unstable version found");
        m.insert("official_suffix", "(official)");
        // corepack.rs additions
        m.insert("corepack_enabled_for", "Corepack enabled for:");
        m.insert("corepack_not_found_for", "Corepack not found for:");
        m.insert("corepack_install_tip", "nvm corepack enable {0}");
        m.insert("system_corepack", "System corepack:");
        m.insert("corepack_no_version", "No version selected. Run 'nvm corepack status <version>' for a specific version.");
        m.insert("no_version_no_current", "No version specified and no current version set");
        m.insert("corepack_enabled_via_npm", "Corepack enabled (via npm) for:");
        m.insert("corepack_enable_failed", "Failed to enable corepack. Run 'nvm install {0} --latest-npm' first.");
        m.insert("npm_not_found", "npm not found for version {0}");
        m.insert("corepack_disabled_for", "Corepack disabled for:");
        m.insert("corepack_maybe_not_enabled", "Corepack may not have been enabled for:");
        m.insert("corepack_usage", "Usage: nvm corepack <enable|disable|status> [version]");
        // completions.rs additions
        m.insert("unsupported_shell", "Unsupported shell: {0}. Supported: bash, zsh, fish, powershell");
        m.insert("completion_hint", "Run 'nvm completion bash' to generate for bash.");
        m.insert("completions_written_bash", "Bash completions written to:");
        m.insert("completions_written_zsh", "Zsh completions written to:");
        m.insert("completions_written_fish", "Fish completions written to:");
        m.insert("completions_written_powershell", "PowerShell completions written to:");
        m.insert("add_to_bashrc", "Add this to your ~/.bashrc:");
        m.insert("add_to_zshrc", "Add this to your ~/.zshrc:");
        m.insert("add_to_fish_config", "Add this to your ~/.config/fish/config.fish:");
        m.insert("add_to_powershell_profile", "Add this to your PowerShell profile:");
        // cli.rs help additions
        m.insert("help_title", "Node Version Manager (Rust) - nvm-rs");
        m.insert("help_usage_line", "Usage: nvm <command> [args]");
        m.insert("help_core_commands", "Core Commands:");
        m.insert("help_alias_commands", "Alias Commands:");
        m.insert("help_env_commands", "Environment Commands:");
        m.insert("help_package_commands", "Package Commands:");
        m.insert("help_info_commands", "Info Commands:");
        m.insert("help_env_vars", "Environment Variables:");
        m.insert("help_special_aliases", "Special Aliases:");
        m.insert("help_desc_install", "Install a version (20, v20.11.0, lts, lts/iron, node, etc.)");
        m.insert("help_desc_uninstall", "Uninstall a version");
        m.insert("help_desc_remove", "Alias for uninstall");
        m.insert("help_desc_use", "Switch to a version");
        m.insert("help_desc_list", "List locally installed versions");
        m.insert("help_desc_remote", "List remote available versions");
        m.insert("help_desc_current", "Show current version");
        m.insert("help_desc_dir", "Show NVM installation and home paths");
        m.insert("help_desc_which", "Show version binary path");
        m.insert("help_desc_run", "Run with a specific version");
        m.insert("help_desc_exec", "Execute command with a specific version");
        m.insert("help_desc_alias", "Set or show aliases");
        m.insert("help_desc_unalias", "Remove an alias");
        m.insert("help_desc_auto", "Auto-switch via .nvmrc/.node-version");
        m.insert("help_desc_deactivate", "Deactivate current version");
        m.insert("help_desc_unload", "Remove NVM from shell config");
        m.insert("help_desc_install_npm", "Upgrade npm to latest");
        m.insert("help_desc_reinstall", "Migrate global packages from a version");
        m.insert("help_desc_version", "Show current node/npm version");
        m.insert("help_desc_version_remote", "Show recent remote versions");
        m.insert("help_desc_mirror", "Set or show mirror");
        m.insert("help_desc_env_nvm_dir", "NVM install directory");
        m.insert("help_desc_special_node", "Latest / stable / unstable");
        m.insert("help_desc_lts_codename", "LTS version (argon/boron/.../iron/jod)");
        m.insert("help_desc_system", "System node");
        m.insert("help_desc_default", "Default version");
        // subcommand help (for nvm <cmd> -h)
        m.insert("help_install_about", "Install a specific Node.js version");
        m.insert("help_install_usage", "Usage: nvm install [OPTIONS] [VERSION]");
        m.insert("help_install_version_arg", "Version number (supports 20, v20.11.0, lts/*, node, stable, etc.)");
        m.insert("help_install_lts", "Install the latest LTS version");
        m.insert("help_install_latest", "Install the latest release");
        m.insert("help_install_lts_newer", "Install latest LTS only if not already installed");
        m.insert("help_install_offline", "Install from cache only (no network)");
        m.insert("help_install_reinstall", "Reinstall global packages from a version after install");
        m.insert("help_install_latest_npm", "Upgrade npm to latest after install");
        m.insert("help_install_source", "Compile and install from source (requires compiler toolchain)");
        m.insert("help_use_about", "Switch to a specific Node.js version");
        m.insert("help_use_usage", "Usage: nvm use [OPTIONS] <VERSION>");
        m.insert("help_use_version_arg", "Version number or alias");
        m.insert("help_use_install_if_missing", "Install the version if it is not installed yet");
        m.insert("help_use_save", "Persist this version as the default");
        m.insert("help_use_use_on_cd", "Enable auto-switch on directory change (installs shell cd hook)");
        m.insert("help_list_about", "List locally installed versions");
        m.insert("help_list_usage", "Usage: nvm list");
        m.insert("help_remote_about", "List remotely available versions");
        m.insert("help_remote_usage", "Usage: nvm remote [OPTIONS] [PAGE]");
        m.insert("help_remote_page_arg", "Page number (1-based, 20 per page)");
        m.insert("help_remote_lts", "Only show LTS versions");
        m.insert("help_remote_lts_old", "Only show LTS versions <= 18");
        m.insert("help_remote_filter", "Filter versions (e.g., \"20\", \"18\", \"16\")");
        m.insert("help_remote_sort", "Sort order: \"desc\" (default) or \"asc\"");
        m.insert("help_uninstall_about", "Uninstall a specific version");
        m.insert("help_uninstall_usage", "Usage: nvm uninstall [OPTIONS] [VERSION]");
        m.insert("help_uninstall_version_arg", "Version number (or --lts to uninstall latest LTS)");
        m.insert("help_uninstall_lts", "Uninstall the latest LTS version");
        m.insert("help_current_about", "Show the currently active version");
        m.insert("help_current_usage", "Usage: nvm current");
        m.insert("help_dir_about", "Show NVM installation and .nvm.rust paths");
        m.insert("help_dir_usage", "Usage: nvm dir");
        m.insert("help_alias_about", "Set or show aliases");
        m.insert("help_alias_usage", "Usage: nvm alias [NAME] [VERSION]");
        m.insert("help_alias_name_arg", "Alias name (shows all aliases if omitted)");
        m.insert("help_alias_version_arg", "Version number");
        m.insert("help_unalias_about", "Remove an alias");
        m.insert("help_unalias_usage", "Usage: nvm unalias <NAME>");
        m.insert("help_unalias_name_arg", "Alias name");
        m.insert("help_mirror_about", "Set or show the download mirror");
        m.insert("help_mirror_usage", "Usage: nvm mirror [MIRROR]");
        m.insert("help_mirror_arg", "Mirror URL (taobao/official/custom URL)");
        m.insert("help_run_about", "Run a script with a specific Node.js version");
        m.insert("help_run_usage", "Usage: nvm run <VERSION> [ARGS]...");
        m.insert("help_run_version_arg", "Version number");
        m.insert("help_run_args_arg", "Script or command to run");
        m.insert("help_exec_about", "Execute a command with a specific Node.js version");
        m.insert("help_exec_usage", "Usage: nvm exec <VERSION> [ARGS]...");
        m.insert("help_exec_version_arg", "Version number");
        m.insert("help_exec_args_arg", "Command and arguments");
        m.insert("help_which_about", "Show the installation path of a version");
        m.insert("help_which_usage", "Usage: nvm which [VERSION]");
        m.insert("help_which_version_arg", "Version number (defaults to current)");
        m.insert("help_auto_about", "Auto-switch based on .nvmrc or .node-version");
        m.insert("help_auto_usage", "Usage: nvm auto [OPTIONS]");
        m.insert("help_auto_silent", "Suppress output (used internally by the cd hook)");
        m.insert("help_deactivate_about", "Deactivate current version (restore PATH)");
        m.insert("help_deactivate_usage", "Usage: nvm deactivate");
        m.insert("help_unload_about", "Remove NVM from shell config");
        m.insert("help_unload_usage", "Usage: nvm unload");
        m.insert("help_install_npm_about", "Upgrade npm to the latest for a version");
        m.insert("help_install_npm_usage", "Usage: nvm install-latest-npm [VERSION]");
        m.insert("help_install_npm_version_arg", "Version number (defaults to current)");
        m.insert("help_reinstall_about", "Migrate global packages from one version to current");
        m.insert("help_reinstall_usage", "Usage: nvm reinstall-packages <FROM>");
        m.insert("help_reinstall_from_arg", "Source version");
        m.insert("help_version_about", "Show current node/npm version info");
        m.insert("help_version_usage", "Usage: nvm version");
        m.insert("help_version_remote_about", "Show remote node/npm version info");
        m.insert("help_version_remote_usage", "Usage: nvm version-remote");
        m.insert("help_cache_about", "Cache management");
        m.insert("help_cache_usage", "Usage: nvm cache <COMMAND>");
        m.insert("help_cache_commands", "Commands:");
        m.insert("help_cache_dir", "Show the cache directory");
        m.insert("help_cache_list", "List cached files");
        m.insert("help_cache_clear", "Clear all cached files");
        m.insert("help_language_about", "Set or show language");
        m.insert("help_language_usage", "Usage: nvm language [LANG]");
        m.insert("help_language_lang_arg", "Language code (en or cn)");
        m.insert("help_proxy_about", "Manage proxy settings");
        m.insert("help_proxy_usage", "Usage: nvm proxy [ACTION]");
        m.insert("help_proxy_action_arg", "Action: on / off (show status if omitted)");
        m.insert("help_completion_about", "Generate shell completions");
        m.insert("help_completion_usage", "Usage: nvm completion [shell]");
        m.insert("help_completion_shell_arg", "Shell type (bash, zsh, fish, powershell)");
        m.insert("help_corepack_about", "Enable/disable corepack for a version");
        m.insert("help_corepack_usage", "Usage: nvm corepack [ACTION] [VERSION]");
        m.insert("help_corepack_action_arg", "Action: enable / disable / status");
        m.insert("help_corepack_version_arg", "Version to enable/disable corepack for");
        m.insert("help_migrate_about", "Migrate installed versions from nvm-sh or nvm-windows");
        m.insert("help_migrate_usage", "Usage: nvm migrate [SOURCE]");
        m.insert("help_migrate_source_arg", "Source: nvm (default) or nvm-windows");
        m.insert("help_options_label", "Options:");
        m.insert("help_arguments_label", "Arguments:");
        m.insert("help_help_flag", "Print help");
        m.insert("help_root_usage", "Usage: nvm [COMMAND]");
        m.insert("help_root_commands", "Commands:");
        m.insert("help_root_print_help", "Print this message or the help of the given subcommand(s)");
        m.insert("help_version_flag", "Print version");
        m
    };

    static ref CN_STRINGS: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("checking_lts", "正在检查最新 LTS 版本...");
        m.insert("checking_latest", "正在检查最新版本...");
        m.insert("checking_remote", "正在检查远程版本...");
        m.insert("no_installed_versions", "还没有安装任何版本。");
        m.insert("run_get_started", "运行 {0} 开始使用");
        m.insert("installing_node", "正在安装 Node.js");
        m.insert("compiling_node", "正在从源码编译 Node.js");
        m.insert("installing_iojs", "正在安装 io.js");
        m.insert("downloading", "正在下载...");
        m.insert("extracting", "正在解压...");
        m.insert("installed_exclaim", "{0} 安装完成！");
        m.insert("already_installed", "{0} 已安装");
        m.insert("compiled", "{0} 从源码编译完成！");
        m.insert("checksum_verified", "✓ 校验通过");
        m.insert("checksum_skipped", "⚠ 已跳过");
        m.insert("checksum_offline", "⊘ 已跳过（离线模式）");
        m.insert("using_cache", "离线模式：使用缓存");
        m.insert("cached_file", "[缓存] 使用缓存文件");
        m.insert("cached_saved", "[缓存] 已保存到缓存");
        m.insert("run_use", "执行：nvm use {0}");
        m.insert("url_label", "下载地址：");
        m.insert("checksum_label", "校验和：");
        m.insert("cache_dir_label", "缓存目录：");
        m.insert("cache_empty", "缓存为空。");
        m.insert("cache_cleared", "缓存已清理，释放 {0}");
        m.insert("cache_files", "{0} 个文件");
        m.insert("cache_total", "总计：{0}");
        m.insert("cache_title", "缓存文件");
        m.insert("nvm_dir_title", "NVM 安装目录");
        m.insert("nvm_dir_path", "NVM_DIR：");
        m.insert("nvm_home_title", "NVM 主目录");
        m.insert("nvm_home_path", ".nvm.rust：");
        m.insert("installed_versions_title", "已安装的 Node.js 版本");
        m.insert("installed_all_title", "已安装的版本");
        m.insert("installed", "已安装 {0} 个");
        m.insert("active", "● 当前版本：{0}");
        m.insert("no_active", "○ 没有激活的版本");
        m.insert("remote_title", "可用的 Node.js 版本");
        m.insert("remote_lts_title", "可用的 LTS 版本");
        m.insert("remote_lts_old_title", "可用的旧 LTS 版本（<= 18）");
        m.insert("remote_iojs_title", "可用的 io.js 版本");
        m.insert("page_info", "第 {0} / {1} 页（显示 {2}-{3}，共 {4} 个）");
        m.insert("prev_page", "← nvm remote {0}");
        m.insert("next_page", "nvm remote {0} →");
        m.insert("version", "版本");
        m.insert("lts_col", "LTS");
        m.insert("codename", "代号");
        m.insert("type_col", "类型");
        m.insert("file_col", "文件");
        m.insert("size_col", "大小");
        m.insert("lts_badge", "✓ LTS");
        m.insert("current", "当前");
        m.insert("system", "系统");
        m.insert("default", "默认");
        m.insert("aliases_title", "别名：");
        m.insert("alias_set", "✓ {0} → {1}");
        m.insert("alias_removed", "✓ 别名已删除：{0}");
        m.insert("alias_not_found", "别名 '{0}' 不存在");
        m.insert("alias_name_empty", "别名名称不能为空");
        m.insert("current_mirror", "当前镜像源：");
        m.insert("mirror_url_empty", "镜像源 URL 不能为空");
        m.insert("mirror_set", "✓ 镜像源已设置为：{0}");
        m.insert("mirror_official", "✓ 镜像源已切换为官方：{0}");
        m.insert("shell_config_removed", "NVM Rust 已从 shell 配置中移除");
        m.insert("not_installed", "版本 {0} 未安装");
        m.insert("cannot_resolve", "无法解析版本：{0}");
        m.insert("uninstall_warning", "警告：正在卸载当前激活的版本");
        m.insert("uninstalling", "正在卸载 {0}...");
        m.insert("uninstalled", "✓ 已卸载 {0}");
        m.insert("current_version", "▶ 当前版本：{0}");
        m.insert("which_path", "{0}");
        m.insert("auto_switch", "▶ 自动切换到 {0}（来自 {1}）");
        m.insert("no_nvmrc", "ℹ 未找到 .nvmrc 或 .node-version 文件");
        m.insert("deactivated", "已取消激活当前 Node.js 版本");
        m.insert("upgrading_npm", "正在升级 {0} 的 npm");
        m.insert("npm_upgraded", "npm 升级成功");
        m.insert("npm_upgrade_failed", "npm 升级失败");
        m.insert("npm_upgrade_retry_npx", "第一次尝试失败，正在通过 npx 重试（下载临时 npm@latest 到独立目录）...");
        m.insert("reinstall_packages", "正在从 {0} 重新安装包");
        m.insert("reinstall_complete", "✓ 重新安装完成");
        m.insert("migrating_packages", "正在迁移包从 {0} → {1}");
        m.insert("migration_failed", "包迁移失败：{0}");
        m.insert("version_info", "版本信息");
        m.insert("remote_version_info", "远程版本信息");
        m.insert("latest_lts", "最新 LTS");
        m.insert("latest_stable", "最新稳定版");
        m.insert("source_configure", "./configure --prefix={0}");
        m.insert("source_make", "make -j{0}");
        m.insert("source_install", "make install");
        m.insert("source_extract", "正在解压源码...");
        m.insert("source_npm_fetch", "源码构建不包含 npm，正在下载预构建版本");
        m.insert("downloading_npm", "正在下载 npm tarball...");
        m.insert("iojs_source_unsupported", "io.js 不支持源码编译");
        m.insert("offline_no_cache", "缓存中找不到文件：{0}\n请去掉 --offline 从网络下载。");
        m.insert("offline_source_no_cache", "缓存中找不到源码包：{0}\n请去掉 --offline 从网络下载。");
        m.insert("lang_set", "✓ 语言已设置为 {0}");
        m.insert("lang_current", "▶ 当前语言：{0}");
        m.insert("lang_usage", "用法：nvm language <en|cn>");
        m.insert("lang_unknown", "未知语言：{0}（使用 'en' 或 'cn'）");
        m.insert("node_label", "node：");
        m.insert("npm_label", "npm：");
        m.insert("testing_connectivity", "正在测试连接性...");
        m.insert("no_versions_found", "未找到任何版本。");
        m.insert("proxy_no_system_proxy", "未检测到系统代理。");
        m.insert("proxy_set_env_vars", "请先设置 HTTPS_PROXY / HTTP_PROXY 环境变量");
        m.insert("proxy_test_google_ok", "google ✓");
        m.insert("proxy_test_google_fail", "google ✗");
        m.insert("proxy_status_title", "代理状态");
        // commands.rs additions
        m.insert("fetching_remote", "正在获取远程版本...");
        m.insert("installing_product", "正在安装 {0}");
        m.insert("compiling_product", "正在编译 {0}");
        m.insert("installed_msg", "{0} 安装完成！");
        m.insert("specify_version_lts_latest", "请指定版本，或使用 --lts / --latest");
        m.insert("specify_version", "请指定版本，或在含有 .nvmrc / .node-version / package.json 的目录中运行");
        m.insert("extract_source_failed", "解压源码包失败");
        m.insert("configure_failed", "configure 失败");
        m.insert("make_failed", "make 失败");
        m.insert("make_install_failed", "make install 失败");
        m.insert("version_no_npm", "版本 {0} 没有 npm");
        m.insert("packages_migrated", "已迁移 {0} 个包");
        m.insert("source_not_installed", "源版本 {0} 未安装");
        m.insert("target_not_installed", "目标版本 {0} 未安装");
        m.insert("cannot_fetch_versions", "无法获取版本列表");
        m.insert("cannot_determine_lts", "无法确定最新 LTS 版本");
        m.insert("cannot_determine_latest", "无法确定最新版本");
        m.insert("cannot_find_url", "找不到 {0} 的下载地址");
        m.insert("no_iojs_match", "未找到匹配 {0} 的 io.js 版本");
        m.insert("cannot_resolve_iojs", "无法解析 io.js 版本：{0}");
        m.insert("npm_download_failed", "从 {0} 下载 npm 失败");
        m.insert("uninstalling_label", "正在卸载");
        m.insert("now_using", "现在使用");
        m.insert("system_node", "系统 Node.js");
        m.insert("version_not_installed_installing", "{0} 未安装，正在安装...");
        m.insert("install_failed", "安装版本 {0} 失败");
        m.insert("not_installed_run_install", "版本 {0} 未安装。请先运行 'nvm install {0}'。");
        m.insert("no_installed_lts", "未找到已安装的 LTS 版本。请用 'nvm install --lts' 安装一个。");
        m.insert("package_failed_code", "失败（退出码 {0}）");
        m.insert("reinstall_failed_list", "部分包迁移失败：{0}");
        m.insert("now_using_node", "现在使用 Node.js");
        m.insert("now_using_iojs", "现在使用 io.js");
        m.insert("tip_label", "提示：");
        m.insert("tip_apply_shell", "执行 source ~/.bashrc（或 ~/.zshrc）以在当前 shell 生效");
        m.insert("no_active_use", "没有激活的版本，运行 'nvm use <version>' 设置一个");
        m.insert("no_current_version_set", "未设置当前版本");
        m.insert("specify_command", "请指定要运行的命令");
        m.insert("specify_version_or_lts", "请指定版本或使用 --lts 标志");
        m.insert("found_engines_node", "在 package.json 中找到 engines.node：");
        m.insert("no_nvmrc_found", "未找到 .nvmrc、.node-version 或包含 engines.node 的 package.json");
        m.insert("found_nvmrc", "在以下位置找到 .nvmrc：");
        m.insert("found_node_version", "在以下位置找到 .node-version：");
        m.insert("system_node_label", "系统 node：");
        m.insert("active_node_label", "激活的 node：");
        m.insert("no_active_version_set", "未设置激活的版本");
        m.insert("latest_remote_versions", "最新的远程版本");
        m.insert("pagination_summary", "第 {0}/{1} 页    共 {2} 个    显示 {3}-{4}");
        m.insert("proxy_test_baidu_ok", "baidu ✓");
        m.insert("proxy_test_baidu_fail", "baidu ✗");
        m.insert("proxy_enabled", "代理已启用。");
        m.insert("proxy_will_be_used", "下载将使用系统代理。");
        m.insert("neither_reachable", "baidu 和 google 均不可达。");
        m.insert("check_proxy_settings", "请先检查代理设置（HTTPS_PROXY / HTTP_PROXY）");
        m.insert("proxy_disabled", "代理已禁用。使用直连。");
        m.insert("unknown_action", "未知操作：{0}\n使用 'on'、'off'，或省略以查看状态");
        m.insert("proxy_active", "代理已启用；下载使用系统代理。");
        m.insert("proxy_on_no_env", "代理已开启但未检测到系统代理环境变量。");
        m.insert("proxy_off_direct", "代理已关闭；下载使用直连。");
        m.insert("usage_label", "用法：");
        m.insert("proxy_usage_hint", "nvm proxy on   /   nvm proxy off");
        m.insert("not_set", "未设置");
        // nvm use --save
        m.insert("default_saved", "默认版本已设为 {0}");
        // nvm use --use-on-cd
        m.insert("use_on_cd_enabled", "cd 自动切换已启用（已安装 shell cd hook）");
        // nvm migrate
        m.insert("migrate_scanning", "正在扫描 {0} 中已安装的版本...");
        m.insert("migrate_no_versions", "源目录中未找到已安装的版本。");
        m.insert("migrate_imported", "已导入 {0}");
        m.insert("migrate_skipped", "{0} 已存在，已跳过");
        m.insert("migrate_failed", "导入 {0} 失败");
        m.insert("migrate_source_not_found", "未找到迁移源 '{0}'。请设置 NVM_SH_HOME 环境变量指向非默认位置的 nvm-sh 安装目录。");
        m.insert("migrate_default_set", "默认版本已设为 {0}");
        m.insert("migrate_summary", "已迁移 {0} 个版本，跳过 {1} 个");
        m.insert("language_set_label", "语言已设置为：");
        m.insert("current_language_label", "当前语言：");
        m.insert("nvm_language_hint", "nvm language <en|cn>");
        // config.rs additions
        m.insert("alias_not_exist_msg", "别名 '{0}' 不存在");
        m.insert("no_aliases", "尚未定义别名。");
        m.insert("no_default_version", "未设置默认版本");
        m.insert("system_node_not_found", "未找到系统 node");
        m.insert("unknown_lts_alias", "未知的 LTS 别名：{0}");
        m.insert("no_matching_version", "未找到匹配的已安装版本（前缀={0}）");
        m.insert("no_unstable", "未找到不稳定版本");
        m.insert("official_suffix", "（官方）");
        // corepack.rs additions
        m.insert("corepack_enabled_for", "已为以下版本启用 corepack：");
        m.insert("corepack_not_found_for", "未找到 corepack：");
        m.insert("corepack_install_tip", "nvm corepack enable {0}");
        m.insert("system_corepack", "系统 corepack：");
        m.insert("corepack_no_version", "未选择版本。运行 'nvm corepack status <version>' 查看指定版本。");
        m.insert("no_version_no_current", "未指定版本且未设置当前版本");
        m.insert("corepack_enabled_via_npm", "已通过 npm 启用 corepack：");
        m.insert("corepack_enable_failed", "启用 corepack 失败。请先运行 'nvm install {0} --latest-npm'。");
        m.insert("npm_not_found", "版本 {0} 未找到 npm");
        m.insert("corepack_disabled_for", "已为以下版本禁用 corepack：");
        m.insert("corepack_maybe_not_enabled", "可能未为以下版本启用 corepack：");
        m.insert("corepack_usage", "用法：nvm corepack <enable|disable|status> [version]");
        // completions.rs additions
        m.insert("unsupported_shell", "不支持的 shell：{0}。支持：bash、zsh、fish、powershell");
        m.insert("completion_hint", "运行 'nvm completion bash' 生成 bash 补全。");
        m.insert("completions_written_bash", "Bash 补全已写入：");
        m.insert("completions_written_zsh", "Zsh 补全已写入：");
        m.insert("completions_written_fish", "Fish 补全已写入：");
        m.insert("completions_written_powershell", "PowerShell 补全已写入：");
        m.insert("add_to_bashrc", "将其添加到 ~/.bashrc：");
        m.insert("add_to_zshrc", "将其添加到 ~/.zshrc：");
        m.insert("add_to_fish_config", "将其添加到 ~/.config/fish/config.fish：");
        m.insert("add_to_powershell_profile", "将其添加到 PowerShell 配置文件：");
        // cli.rs help additions
        m.insert("help_title", "Node 版本管理器（Rust）- nvm-rs");
        m.insert("help_usage_line", "用法：nvm <command> [参数]");
        m.insert("help_core_commands", "核心命令：");
        m.insert("help_alias_commands", "别名命令：");
        m.insert("help_env_commands", "环境命令：");
        m.insert("help_package_commands", "包管理命令：");
        m.insert("help_info_commands", "信息命令：");
        m.insert("help_env_vars", "环境变量：");
        m.insert("help_special_aliases", "特殊别名：");
        m.insert("help_desc_install", "安装版本（20, v20.11.0, lts, lts/iron, node 等）");
        m.insert("help_desc_uninstall", "卸载版本");
        m.insert("help_desc_remove", "卸载的别名");
        m.insert("help_desc_use", "切换到指定版本");
        m.insert("help_desc_list", "列出本地已安装版本");
        m.insert("help_desc_remote", "列出远程可用版本");
        m.insert("help_desc_current", "显示当前版本");
        m.insert("help_desc_dir", "显示 NVM 安装路径和主目录");
        m.insert("help_desc_which", "显示版本二进制路径");
        m.insert("help_desc_run", "使用指定版本运行");
        m.insert("help_desc_exec", "使用指定版本执行命令");
        m.insert("help_desc_alias", "设置或查看别名");
        m.insert("help_desc_unalias", "删除别名");
        m.insert("help_desc_auto", "通过 .nvmrc/.node-version 自动切换");
        m.insert("help_desc_deactivate", "停用当前版本");
        m.insert("help_desc_unload", "从 shell 配置中移除 NVM");
        m.insert("help_desc_install_npm", "升级 npm 到最新版");
        m.insert("help_desc_reinstall", "从指定版本迁移全局包");
        m.insert("help_desc_version", "显示当前 node/npm 版本");
        m.insert("help_desc_version_remote", "显示最近的远程版本");
        m.insert("help_desc_mirror", "设置或查看镜像源");
        m.insert("help_desc_env_nvm_dir", "NVM 安装目录");
        m.insert("help_desc_special_node", "最新 / 稳定 / 不稳定");
        m.insert("help_desc_lts_codename", "LTS 版本（argon/boron/.../iron/jod）");
        m.insert("help_desc_system", "系统 node");
        m.insert("help_desc_default", "默认版本");
        // subcommand help (for nvm <cmd> -h)
        m.insert("help_install_about", "安装指定 Node.js 版本");
        m.insert("help_install_usage", "用法：nvm install [选项] [VERSION]");
        m.insert("help_install_version_arg", "版本号（支持 20、v20.11.0、lts/*、node、stable 等）");
        m.insert("help_install_lts", "安装最新 LTS 版本");
        m.insert("help_install_latest", "安装最新发布版本");
        m.insert("help_install_lts_newer", "仅在未安装时安装最新 LTS 版本");
        m.insert("help_install_offline", "仅从缓存安装（不联网）");
        m.insert("help_install_reinstall", "安装后从指定版本重装全局包");
        m.insert("help_install_latest_npm", "安装后升级 npm 到最新版");
        m.insert("help_install_source", "从源码编译安装（需要编译工具链）");
        m.insert("help_use_about", "切换到指定 Node.js 版本");
        m.insert("help_use_usage", "用法：nvm use [选项] <VERSION>");
        m.insert("help_use_version_arg", "版本号或别名");
        m.insert("help_use_install_if_missing", "未安装时自动安装该版本");
        m.insert("help_use_save", "将此版本持久化为默认版本");
        m.insert("help_use_use_on_cd", "启用 cd 切目录时自动切换版本（安装 shell cd hook）");
        m.insert("help_list_about", "列出本地已安装版本");
        m.insert("help_list_usage", "用法：nvm list");
        m.insert("help_remote_about", "列出远程可用版本");
        m.insert("help_remote_usage", "用法：nvm remote [选项] [PAGE]");
        m.insert("help_remote_page_arg", "页码（从 1 开始，每页 20 条）");
        m.insert("help_remote_lts", "仅显示 LTS 版本");
        m.insert("help_remote_lts_old", "仅显示 LTS 版本 <= 18");
        m.insert("help_remote_filter", "过滤版本（如 \"20\"、\"18\"、\"16\"）");
        m.insert("help_remote_sort", "排序方式：\"desc\"（默认）或 \"asc\"");
        m.insert("help_uninstall_about", "卸载指定版本");
        m.insert("help_uninstall_usage", "用法：nvm uninstall [选项] [VERSION]");
        m.insert("help_uninstall_version_arg", "版本号（或用 --lts 卸载最新 LTS）");
        m.insert("help_uninstall_lts", "卸载最新 LTS 版本");
        m.insert("help_current_about", "显示当前激活的版本");
        m.insert("help_current_usage", "用法：nvm current");
        m.insert("help_dir_about", "显示 NVM 安装路径和 .nvm.rust 路径");
        m.insert("help_dir_usage", "用法：nvm dir");
        m.insert("help_alias_about", "设置或查看别名");
        m.insert("help_alias_usage", "用法：nvm alias [NAME] [VERSION]");
        m.insert("help_alias_name_arg", "别名名称（省略则显示所有别名）");
        m.insert("help_alias_version_arg", "版本号");
        m.insert("help_unalias_about", "删除别名");
        m.insert("help_unalias_usage", "用法：nvm unalias <NAME>");
        m.insert("help_unalias_name_arg", "别名名称");
        m.insert("help_mirror_about", "设置或查看下载镜像");
        m.insert("help_mirror_usage", "用法：nvm mirror [MIRROR]");
        m.insert("help_mirror_arg", "镜像地址（taobao/official/自定义 URL）");
        m.insert("help_run_about", "用指定 Node.js 版本运行脚本");
        m.insert("help_run_usage", "用法：nvm run <VERSION> [ARGS]...");
        m.insert("help_run_version_arg", "版本号");
        m.insert("help_run_args_arg", "脚本或要运行的命令");
        m.insert("help_exec_about", "用指定 Node.js 版本执行命令");
        m.insert("help_exec_usage", "用法：nvm exec <VERSION> [ARGS]...");
        m.insert("help_exec_version_arg", "版本号");
        m.insert("help_exec_args_arg", "命令及其参数");
        m.insert("help_which_about", "显示某版本的安装路径");
        m.insert("help_which_usage", "用法：nvm which [VERSION]");
        m.insert("help_which_version_arg", "版本号（默认为当前版本）");
        m.insert("help_auto_about", "根据 .nvmrc 或 .node-version 自动切换");
        m.insert("help_auto_usage", "用法：nvm auto [选项]");
        m.insert("help_auto_silent", "静默输出（由 cd hook 内部使用）");
        m.insert("help_deactivate_about", "停用当前版本（恢复 PATH）");
        m.insert("help_deactivate_usage", "用法：nvm deactivate");
        m.insert("help_unload_about", "从 shell 配置中移除 NVM");
        m.insert("help_unload_usage", "用法：nvm unload");
        m.insert("help_install_npm_about", "为某版本升级 npm 到最新版");
        m.insert("help_install_npm_usage", "用法：nvm install-latest-npm [VERSION]");
        m.insert("help_install_npm_version_arg", "版本号（默认为当前版本）");
        m.insert("help_reinstall_about", "从指定版本迁移全局包到当前版本");
        m.insert("help_reinstall_usage", "用法：nvm reinstall-packages <FROM>");
        m.insert("help_reinstall_from_arg", "源版本");
        m.insert("help_version_about", "显示当前 node/npm 版本信息");
        m.insert("help_version_usage", "用法：nvm version");
        m.insert("help_version_remote_about", "显示远程 node/npm 版本信息");
        m.insert("help_version_remote_usage", "用法：nvm version-remote");
        m.insert("help_cache_about", "缓存管理");
        m.insert("help_cache_usage", "用法：nvm cache <COMMAND>");
        m.insert("help_cache_commands", "子命令：");
        m.insert("help_cache_dir", "显示缓存目录");
        m.insert("help_cache_list", "列出缓存文件");
        m.insert("help_cache_clear", "清除所有缓存文件");
        m.insert("help_language_about", "设置或查看语言");
        m.insert("help_language_usage", "用法：nvm language [LANG]");
        m.insert("help_language_lang_arg", "语言代码（en 或 cn）");
        m.insert("help_proxy_about", "管理代理设置");
        m.insert("help_proxy_usage", "用法：nvm proxy [ACTION]");
        m.insert("help_proxy_action_arg", "操作：on / off（省略则显示状态）");
        m.insert("help_completion_about", "生成 shell 补全");
        m.insert("help_completion_usage", "用法：nvm completion [shell]");
        m.insert("help_completion_shell_arg", "shell 类型（bash、zsh、fish、powershell）");
        m.insert("help_corepack_about", "为某版本启用/禁用 corepack");
        m.insert("help_corepack_usage", "用法：nvm corepack [ACTION] [VERSION]");
        m.insert("help_corepack_action_arg", "操作：enable / disable / status");
        m.insert("help_corepack_version_arg", "要启用/禁用 corepack 的版本");
        m.insert("help_migrate_about", "从 nvm-sh 或 nvm-windows 迁移已安装的版本");
        m.insert("help_migrate_usage", "用法：nvm migrate [SOURCE]");
        m.insert("help_migrate_source_arg", "来源：nvm（默认）或 nvm-windows");
        m.insert("help_options_label", "选项：");
        m.insert("help_arguments_label", "参数：");
        m.insert("help_help_flag", "显示帮助");
        m.insert("help_root_usage", "用法：nvm [COMMAND]");
        m.insert("help_root_commands", "子命令：");
        m.insert("help_root_print_help", "显示此消息或指定子命令的帮助");
        m.insert("help_version_flag", "显示版本号");
        m
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lang_from_str() {
        assert_eq!(Lang::from_str("en"), Some(Lang::En));
        assert_eq!(Lang::from_str("EN"), Some(Lang::En));
        assert_eq!(Lang::from_str("english"), Some(Lang::En));
        assert_eq!(Lang::from_str("cn"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("CN"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("zh"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("zh-cn"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("chinese"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("中文"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("invalid"), None);
        assert_eq!(Lang::from_str(""), None);
    }

    #[test]
    fn test_lang_as_str() {
        assert_eq!(Lang::En.as_str(), "en");
        assert_eq!(Lang::Cn.as_str(), "cn");
    }

    #[test]
    fn test_lang_display_name() {
        assert_eq!(Lang::En.display_name(), "English");
        assert_eq!(Lang::Cn.display_name(), "中文");
    }

    #[test]
    fn test_lang_default() {
        assert_eq!(Lang::default(), Lang::En);
    }
}
