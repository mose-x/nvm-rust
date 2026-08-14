use anyhow::Result;

use colored::Colorize;

mod cli;
mod commands;
mod completions;
mod config;
mod corepack;
mod download;
mod extract;
mod i18n;
mod proxy;
mod shim;
mod system;
mod utils;

use clap::Parser;
use cli::{CacheAction, Cli, Commands};

fn main() -> Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: geteuid() is a read-only system call with no preconditions.
        let is_root = unsafe { libc::geteuid() } == 0;
        let allow_root = std::env::var("NVM_ALLOW_ROOT")
            .map(|v| v == "1")
            .unwrap_or(false);
        if is_root && !allow_root {
            eprintln!(
                "{} {}",
                "⚠".yellow().bold(),
                crate::i18n::T("root_not_supported")
            );
            eprintln!("  {}", crate::i18n::T("root_hint"));
            eprintln!("  {}", crate::i18n::T("root_force_hint"));
            std::process::exit(1);
        }
    }

    system::os_check();
    system::ensure_nvm_dir()?;

    // Intercept -h/--help/help so clap's compile-time (English) help is bypassed
    // and we render i18n-aware help instead.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(action) = cli::intercept_help(&argv) {
        match action {
            cli::HelpAction::Root => cli::print_root_help(),
            cli::HelpAction::Command(name) => cli::print_command_help(&name),
            cli::HelpAction::Version => println!("nvm {}", env!("CARGO_PKG_VERSION")),
        }
        return Ok(());
    }

    // Use try_parse so we can intercept clap's English error messages and
    // render i18n-aware ones for the most common error kinds (unknown
    // command, unknown flag). Other errors (missing required args, invalid
    // values) still fall through to clap's default handler.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument => {
                    eprintln!(
                        "{} {}",
                        "⚠".yellow().bold(),
                        crate::i18n::T("error_invalid_command")
                    );
                    eprintln!(
                        "  {} {}",
                        crate::i18n::T("tip_label"),
                        crate::i18n::T("error_run_help")
                    );
                    std::process::exit(2);
                }
                _ => e.exit(),
            }
        }
    };

    match cli.command {
        None => {
            cli::print_help();
            Ok(())
        }
        Some(cmd) => match cmd {
            Commands::Install {
                version,
                lts,
                latest,
                lts_newer,
                offline,
                reinstall_packages_from,
                latest_npm,
                latest_yarn,
                latest_pnpm,
                source,
                no_gpg_verify,
            } => commands::install(commands::InstallConfig {
                version,
                lts,
                latest,
                lts_newer,
                offline,
                reinstall_packages_from,
                latest_npm,
                latest_yarn,
                latest_pnpm,
                source,
                no_gpg_verify,
            }),
            Commands::Use {
                version,
                install_if_missing,
                save,
                use_on_cd,
            } => commands::use_version(version.as_deref(), install_if_missing, save, use_on_cd),
            Commands::List => commands::list_versions(),
            Commands::Remote {
                lts,
                lts_old,
                filter,
                sort,
                page,
            } => commands::remote_versions(lts, lts_old, filter.as_deref(), sort.as_deref(), page),
            Commands::Uninstall {
                version,
                lts,
                latest,
            } => match (version, lts, latest) {
                (Some(v), false, false) => commands::uninstall(&v),
                (None, true, false) => commands::uninstall_latest_lts(),
                (None, false, true) => commands::uninstall_latest(),
                _ => anyhow::bail!("{}", crate::i18n::T("specify_version_or_lts")),
            },
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
            Commands::InstallNpm { version } => commands::install_latest_npm(version.as_deref()),
            Commands::InstallYarn { version } => commands::install_latest_yarn(version.as_deref()),
            Commands::InstallPnpm { version } => commands::install_latest_pnpm(version.as_deref()),
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
            Commands::Corepack { action, version } => {
                corepack::handle_corepack(action.as_deref(), version.as_deref())
            }
            Commands::Migrate { source } => commands::cmd_migrate(&source),
            Commands::Upgrade {
                check,
                force,
                from_gitee,
                from_mirror,
                rollback,
            } => commands::upgrade(check, force, from_gitee, from_mirror, rollback),
        },
    }
}
