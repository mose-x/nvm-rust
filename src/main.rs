use anyhow::Result;

use colored::Colorize;

mod alias;
mod cli;
mod commands;
mod completions;
mod config;
mod corepack;
mod download;
mod extract;
mod fs_util;
mod i18n;
mod lock;
mod lts;
mod mirror;
mod project_detect;
mod proxy;
mod ps_repair;
mod semver_range;
mod shell_config;
mod shim;
mod system;
mod term;
// Test-only: the loopback HTTP server that `download.rs` and `system.rs` tests
// use to exercise the real resume and signature-fetch paths. Kept out of the
// release binary.
#[cfg(test)]
mod test_http;
mod utils;

use clap::Parser;
use cli::{CacheAction, Cli, Commands};

/// Whether a panic message is std's "failed printing to stdout" for a pipe the
/// consumer closed, as opposed to some other write failure that must stay
/// visible.
///
/// The match deliberately avoids the OS error *text*: Windows localizes it, so
/// a Chinese Windows reports `failed printing to stdout: 管道正在被关闭。
/// (os error 232)` and a naive `"Broken pipe"` check silently misses it. The
/// prefix comes from std itself and is not localized; the numeric codes are
/// stable per platform (Unix `EPIPE` = 32, Windows `ERROR_BROKEN_PIPE` = 109
/// and `ERROR_NO_DATA` = 232).
fn is_closed_pipe_panic(msg: &str) -> bool {
    msg.contains("failed printing to stdout")
        && ["(os error 32)", "(os error 109)", "(os error 232)"]
            .iter()
            .any(|code| msg.contains(code))
}

/// Make "the reader went away" a silent exit instead of a panic with a stack
/// trace.
///
/// Rust's runtime installs `SIG_IGN` for `SIGPIPE` so that `write()` returns
/// `EPIPE` rather than killing the process — which is exactly why `println!`
/// turns a closed pipe into a panic. Every normal Unix tool dies by signal
/// here, so `nvm list | head -1` should behave like `ls | head -1`.
///
/// Must be called before anything writes to stdout.
fn install_pipe_handling() {
    #[cfg(unix)]
    {
        // SAFETY: `signal(2)` with `SIG_DFL` has no preconditions and no
        // meaningful failure mode. `libc` is already a dependency and `main`
        // below calls into it for `geteuid()`.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }

    // Fallback for everything a signal cannot cover: Windows has no `SIGPIPE`
    // at all, and even on Unix a failed write to a closed *file* surfaces as an
    // error rather than a signal.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if is_closed_pipe_panic(payload) {
            // The consumer stopped reading. There is nothing left to report and
            // no work worth unwinding for, so exit quietly. Any other panic
            // keeps the default hook and its full diagnostic.
            std::process::exit(0);
        }
        previous_hook(info);
    }));
}

fn main() -> Result<()> {
    install_pipe_handling();

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

    system::os_check()?;
    system::ensure_nvm_dir()?;

    // Check config.json ownership — warn (not block) if root-owned.
    // A root-owned config.json means a previous `sudo nvm` wrote it, and
    // the current non-root user can't update it. nvm continues in read-only
    // mode (writes fail silently), but the user should fix ownership.
    let nvm_dir = system::get_nvm_dir();
    let config_path = nvm_dir.join(system::CONFIG_FILE);
    if config_path.exists() {
        if let Ok(meta) = std::fs::metadata(&config_path) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() == 0 {
                    eprintln!(
                        "  {} config.json is root-owned. Fix: sudo chown $(whoami) {}",
                        "⚠".yellow().bold(),
                        config_path.display()
                    );
                }
            }
            #[cfg(not(unix))]
            {
                let _ = meta;
            }
        }
    }

    // Recover from interrupted upgrade (crash during swap_binary).
    // If .swap-pending exists in the nvm bin dir, a previous upgrade was
    // killed mid-swap. Try to finish the swap or restore the .bak.
    commands::check_swap_recovery();

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
                self_uninstall,
                all,
            } => {
                if self_uninstall {
                    commands::uninstall_self()
                } else if all {
                    commands::uninstall_all()
                } else {
                    match (version, lts, latest) {
                        (Some(v), false, false) => commands::uninstall(&v),
                        (None, true, false) => commands::uninstall_latest_lts(),
                        (None, false, true) => commands::uninstall_latest(),
                        _ => anyhow::bail!("{}", crate::i18n::T("uninstall_hint")),
                    }
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
            Commands::Refresh => commands::refresh(),
            Commands::Init { shell } => commands::init(shell.as_deref()),
            Commands::Doctor { fix, network } => commands::doctor(fix, network),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::is_closed_pipe_panic;

    #[test]
    fn closed_pipe_panic_recognised_across_platforms_and_locales() {
        // Unix EPIPE, plus both Windows codes. The Chinese Windows message is
        // the important one: the OS error *text* is localized, so a predicate
        // matching "Broken pipe" would pass every English-locale test and still
        // fail in the field.
        for msg in [
            "failed printing to stdout: Broken pipe (os error 32)",
            "failed printing to stdout: The pipe is being closed. (os error 232)",
            "failed printing to stdout: 管道正在被关闭。 (os error 232)",
            "failed printing to stdout: The pipe has been ended. (os error 109)",
        ] {
            assert!(
                is_closed_pipe_panic(msg),
                "must be treated as a closed pipe: {msg}"
            );
        }
    }

    #[test]
    fn other_stdout_failures_are_not_swallowed() {
        // The hook exits silently, so it must stay narrowly scoped: a genuine
        // write failure still has to reach the user with a full diagnostic.
        for msg in [
            "failed printing to stdout: No space left on device (os error 28)",
            "failed printing to stdout: Permission denied (os error 13)",
            "failed printing to stderr: Broken pipe (os error 32)",
            "index out of bounds: the len is 0 but the index is 1",
            "",
        ] {
            assert!(!is_closed_pipe_panic(msg), "must not be swallowed: {msg:?}");
        }
    }
}
