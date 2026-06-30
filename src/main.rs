use anyhow::Result;

mod cli;
mod commands;
mod completions;
mod config;
mod corepack;
mod download;
mod extract;
mod i18n;
mod proxy;
mod system;
mod utils;

use cli::{CacheAction, Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    system::os_check();
    system::ensure_nvm_dir()?;

    // Intercept -h/--help/help so clap's compile-time (English) help is bypassed
    // and we render i18n-aware help instead.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(action) = cli::intercept_help(&argv) {
        match action {
            cli::HelpAction::Root => cli::print_root_help(),
            cli::HelpAction::Command(name) => cli::print_command_help(&name),
        }
        return Ok(());
    }

    match Cli::parse().command {
        None => {
            cli::print_help();
            Ok(())
        }
        Some(cmd) => match cmd {
            Commands::Install { version, lts, latest, lts_newer, offline, reinstall_packages_from, latest_npm, source } => {
                commands::install(version, lts, latest, lts_newer, offline, reinstall_packages_from, latest_npm, source)
            }
            Commands::Use { version, install_if_missing, save, use_on_cd } => commands::use_version(version.as_deref(), install_if_missing, save, use_on_cd),
            Commands::List => commands::list_versions(),
            Commands::Remote { lts, lts_old, filter, sort, page } => commands::remote_versions(lts, lts_old, filter.as_deref(), sort.as_deref(), page),
            Commands::Uninstall { version, lts, latest } => {
                match (version, lts, latest) {
                    (Some(v), false, false) => commands::uninstall(&v),
                    (None, true, false) => commands::uninstall_latest_lts(),
                    (None, false, true) => commands::uninstall_latest(),
                    _ => anyhow::bail!("{}", crate::i18n::T("specify_version_or_lts")),
                }
            }
            Commands::Current => commands::current_version(),
            Commands::Dir => commands::cmd_dir(),
            Commands::Alias { name, version } => match name {
                Some(n) => commands::cmd_set_alias(&n, version.as_deref()),
                None => commands::cmd_list_aliases(),
            },
            Commands::Unalias { name } => commands::cmd_remove_alias(&name),
            Commands::Mirror { mirror } => commands::cmd_mirror(mirror.as_deref()),
            Commands::Run { version, args } => commands::run_version(&version, &args),
            Commands::Exec { version, args } => commands::exec_version(&version, &args),
            Commands::Which { version } => commands::which_version(version.as_deref()),
            Commands::Auto { silent } => commands::auto_switch(silent),
            Commands::Deactivate => commands::deactivate(),
            Commands::Unload => commands::unload(),
            Commands::InstallLatestNpm { version } => {
                commands::install_latest_npm(version.as_deref())
            }
            Commands::ReinstallPackages { from } => commands::reinstall_packages(&from),
            Commands::Version => commands::show_version_info(),
            Commands::VersionRemote => commands::show_remote_version_info(),
            Commands::Cache { action } => match action {
                CacheAction::Dir => commands::cache_dir(),
                CacheAction::List => commands::cache_list(),
                CacheAction::Clear => commands::cache_clear(),
            },
            Commands::Language { lang } => commands::cmd_language(lang.as_deref()),
            Commands::Proxy { action } => commands::cmd_proxy(action.as_deref()),
            Commands::Completion { shell } => completions::generate_completions(shell.as_deref()),
            Commands::Corepack { action, version } => corepack::handle_corepack(action.as_deref(), version.as_deref()),
            Commands::Migrate { source } => commands::cmd_migrate(&source),
        },
    }
}
