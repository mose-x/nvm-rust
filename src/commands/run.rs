//! Run / exec / which commands.
//!
//! `run` launches a Node.js script under a specific version.
//! `exec` runs an arbitrary command with the version's bin dir on PATH.
//! `which` prints the path to the node binary for a version.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::PathBuf;
use std::process::Command;

use super::get_current_version;
use crate::config::resolve_alias;
use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, prepend_to_path, version_bin_dir};

/// Exit the process with the exit status of a child command.
///
/// When the child was terminated by a signal (e.g. SIGINT, SIGTERM), the
/// shell convention is to exit with `128 + signal_number`. The previous
/// `status.code().unwrap_or(1)` collapsed every signal death into exit
/// code `1`, so a script could not distinguish "command failed" from
/// "command was killed" (e.g. by Ctrl-C).
///
/// # Platform behavior
///
/// - **Unix** (`linux`/`macos`): `ExitStatus::code()` returns `None` when the
///   child was killed by a signal. We then read the signal number via
///   `ExitStatusExt::signal()` and emit `128 + signal`, matching the POSIX
///   shell convention (`bash`, `zsh`, `sh` all use this). `signal()` returns
///   `None` only if the process exited normally, which contradicts `code()`
///   returning `None` -- we fall back to `1` defensively in case a platform
///   reports neither.
/// - **Windows**: `ExitStatus::code()` *always* returns `Some` because
///   Windows processes exit with a 32-bit code and have no signal concept.
///   The `#[cfg(not(unix))]` branch below is therefore unreachable in
///   practice; it is kept as a defensive fallback so the function compiles
///   and stays total on any future non-Unix target where `code()` might
///   return `None` (no current such target exists in std).
pub fn exit_with_status(status: std::process::ExitStatus) -> ! {
    #[cfg(unix)]
    let code = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|s| 128 + s).unwrap_or(1)
    });
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(1);
    std::process::exit(code);
}

pub fn run_version(version: &str, args: &[String]) -> Result<()> {
    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let (node_path, bin_dir) = if resolved.starts_with("system:") {
        (PathBuf::from("node"), None)
    } else {
        let bin = version_bin_dir(&nvm_dir.join(&resolved));
        (exe_path(&bin, "node"), Some(bin))
    };

    if !resolved.starts_with("system:") && !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    // Prepend the version's bin dir to PATH so child processes spawned by the
    // script (e.g. `child_process.exec('npm install')`) resolve npm/npx from
    // THIS version, not the parent shell's PATH. Matches `exec_version` and
    // nvm-sh's `nvm run` semantics. Without this, `nvm run 20 app.js` that
    // shells out to `npm` would use a different npm (or none).
    let mut cmd = Command::new(&node_path);
    cmd.args(args);
    if let Some(bin) = bin_dir {
        // `prepend_to_path` always returns a usable PATH string (it falls
        // back to the current PATH when the env var is unset), so there is
        // no error case to guard here -- just set it unconditionally.
        let new_path = prepend_to_path(&bin);
        cmd.env("PATH", new_path);
    }
    let status = cmd.status().context(T("execution_failed"))?;

    exit_with_status(status);
}

pub fn exec_version(version: &str, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("{}", T("specify_command"));
    }

    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let bin_dir = if resolved.starts_with("system:") {
        match crate::utils::find_system_node_path() {
            Some(node_path) => match node_path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => anyhow::bail!("{}", T("system_node_not_found")),
            },
            None => anyhow::bail!("{}", T("system_node_not_found")),
        }
    } else {
        // Verify the requested version is actually installed, so we never
        // silently fall back to a system node found later on PATH.
        let version_dir = nvm_dir.join(&resolved);
        if !version_dir.exists() {
            anyhow::bail!(
                "{}",
                format_t("not_installed_run_install", std::slice::from_ref(&resolved))
            );
        }
        version_bin_dir(&nvm_dir.join(&resolved))
    };

    let cmd = &args[0];
    let cmd_args = &args[1..];

    let new_path = prepend_to_path(&bin_dir);

    // `Command::new(cmd).status()` fails synchronously when `cmd` is not on
    // PATH (or is not an executable). The raw io::Error surfaces as
    // "No such file or directory (os error 2)", which is confusing because it
    // doesn't name the command the user typed. Detect that specific case and
    // bail with an i18n message that includes `cmd`.
    let status = Command::new(cmd)
        .args(cmd_args)
        .env("PATH", &new_path)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{}",
                    format_t("exec_command_not_found", std::slice::from_ref(cmd))
                )
            } else {
                anyhow::Error::new(e).context(T("execution_failed"))
            }
        })?;

    exit_with_status(status);
}

pub fn which_version(version: Option<&str>) -> Result<()> {
    let resolved = match version {
        Some(v) => resolve_alias(v)?,
        None => match get_current_version()? {
            Some(v) => v,
            None => anyhow::bail!("{}", T("no_current_version_set")),
        },
    };

    if resolved.starts_with("system:") {
        if let Some(node_path) = crate::utils::find_system_node_path() {
            println!("{}", node_path.display().to_string().white().bold());
            return Ok(());
        }
        anyhow::bail!("{}", T("system_node_not_found"));
    }

    let nvm_dir = get_nvm_dir();
    let node_path = exe_path(&version_bin_dir(&nvm_dir.join(&resolved)), "node");

    if !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    println!("{}", node_path.display().to_string().white().bold());
    Ok(())
}
