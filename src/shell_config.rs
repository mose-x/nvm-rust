use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::i18n::T;
use crate::system::get_nvm_dir;
use crate::utils::{atomic_write, backup_file};

pub fn detect_shell_config() -> Option<String> {
    let home = crate::system::get_home_dir();
    if home == "." {
        return None;
    }
    let home_path = PathBuf::from(&home);

    // On Windows the POSIX rc files (.bashrc/.zshrc/...) don't exist by
    // default — the shell integration target is the PowerShell profile.
    // Probe both PowerShell 7 (`Documents\PowerShell\`) and Windows
    // PowerShell 5.1 (`Documents\WindowsPowerShell\`) and prefer an existing
    // profile so we don't clobber one the user doesn't source. If neither
    // exists, fall back to the PS7 path (created on first write).
    if cfg!(windows) {
        let docs = home_path.join("Documents");
        let candidates = [
            docs.join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
            docs.join("WindowsPowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        ];
        for c in &candidates {
            if c.exists() {
                return Some(c.to_string_lossy().into_owned());
            }
        }
        return Some(candidates[0].to_string_lossy().into_owned());
    }

    // Use PathBuf::join (not `format!("{}/{}", ...)`) so paths stay canonical
    // — mixed `home/foo` separators would still work for `exists()` on Unix
    // but break string comparison against `PathBuf::display()` elsewhere.
    let fish_config = home_path.join(".config").join("fish").join("config.fish");
    let candidates: [PathBuf; 5] = [
        home_path.join(".zshrc"),
        home_path.join(".bashrc"),
        home_path.join(".bash_profile"),
        home_path.join(".profile"),
        fish_config,
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.to_string_lossy().into_owned());
        }
    }
    Some(home_path.join(".bashrc").to_string_lossy().into_owned())
}

/// Detect the shell type from the config file path.
fn detect_shell_type(config_path: &str) -> &'static str {
    if cfg!(windows) && (config_path.ends_with(".ps1") || config_path.contains("PowerShell")) {
        return "powershell";
    }
    if config_path.contains("config.fish") || config_path.contains("/fish/") {
        "fish"
    } else if config_path.ends_with(".zshrc") {
        "zsh"
    } else {
        "bash"
    }
}

/// Generate the cd hook shell code for the given shell type.
fn cd_hook_code(shell_type: &str) -> String {
    match shell_type {
        "zsh" => r#"
# NVM Rust - use-on-cd
autoload -Uz add-zsh-hook
__nvm_use_on_cd() {
    if [[ "$PWD" != "$__NVM_PREV_DIR" ]]; then
        __NVM_PREV_DIR="$PWD"
        nvm auto --silent 2>/dev/null
    fi
}
add-zsh-hook precmd __nvm_use_on_cd
"#
        .to_string(),
        "fish" => r#"
# NVM Rust - use-on-cd
function __nvm_use_on_cd --on-variable PWD
    nvm auto --silent 2>/dev/null
end
"#
        .to_string(),
        "powershell" => r#"
# NVM Rust - use-on-cd
# Wrap the existing prompt so `nvm auto` runs on directory change, mirroring
# bash's PROMPT_COMMAND. The guard prevents double-wrapping across reloads;
# `nvm unload` removes this block wholesale so the original prompt is restored.
if (-not (Test-Path Function:__NVM_ORIG_PROMPT)) {
    if (Test-Path Function:prompt) {
        Rename-Item Function:prompt __NVM_ORIG_PROMPT
    } else {
        function global:__NVM_ORIG_PROMPT { 'PS> ' }
    }
    function global:prompt {
        if ((-not (Test-Path Variable:__NVM_PREV_DIR)) -or ($PWD.Path -ne $__NVM_PREV_DIR)) {
            $global:__NVM_PREV_DIR = $PWD.Path
            nvm auto --silent 2>$null
        }
        __NVM_ORIG_PROMPT
    }
}
"#
        .to_string(),
        _ => r#"
# NVM Rust - use-on-cd
__nvm_use_on_cd() {
    if [[ "$PWD" != "$__NVM_PREV_DIR" ]]; then
        __NVM_PREV_DIR="$PWD"
        nvm auto --silent 2>/dev/null
    fi
}
PROMPT_COMMAND="__nvm_use_on_cd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#
        .to_string(),
    }
}

/// Remove all nvm-rust-managed lines from a shell config body:
/// the cd-hook blocks (for every shell type, so removal works even if the
/// user switched shells) and the `# NVM Rust` marker / `NVM_HOME=` /
/// `export PATH=...<nvm_dir>...` lines.
///
/// Extracted because both [`update_shell_config`] and
/// [`remove_from_shell_config`] ran the identical strip pass — keeping two
/// copies in sync was error-prone (the line-filter predicate was duplicated
/// verbatim, and a future addition to "what counts as an nvm line" would
/// have to be made in both places).
fn strip_nvm_lines(content: &str, nvm_dir_str: &str) -> String {
    // Remember whether the input ended with a newline so we can re-attach it.
    // `lines()` + `join("\n")` otherwise normalises the trailing newline away,
    // which would make `remove_from_shell_config` needlessly rewrite an
    // already-clean file (dropping its final newline) — breaking idempotency
    // and producing a spurious diff every time the command runs on a clean rc.
    let trailing_newline = content.ends_with('\n');
    let mut content = content.to_string();
    // Remove any previously-written cd hook block as an exact substring.
    // Try all shell types so removal still works if the user switched shells
    // (including bash↔powershell on a dual-boot / WSL-adjacent setup).
    for st in &["bash", "zsh", "fish", "powershell"] {
        let hook = cd_hook_code(st);
        if content.contains(&hook) {
            content = content.replace(&hook, "");
        }
    }
    // Remove marker / NVM_HOME / nvm.rust / PATH-export lines line-by-line.
    // Recognises both POSIX (`export NVM_HOME=`, `export PATH=`) and
    // PowerShell (`$env:NVM_HOME =`, `$env:PATH =`) forms so cleanup works
    // regardless of which shell wrote the lines.
    let mut out: String = content
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("NVM_HOME")
                || l.contains("nvm.rust")
                || l.contains(".nvm.rust")
                || l.contains("# NVM Rust")
                || (l.starts_with("export PATH=") && l.contains(nvm_dir_str))
                || (l.starts_with("$env:PATH") && l.contains(nvm_dir_str)))
        })
        .collect::<Vec<&str>>()
        .join("\n");
    // Re-attach the trailing newline only when there's remaining content —
    // an all-nvm file should become empty, not a lone "\n".
    if trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn update_shell_config(version: &str, use_on_cd: bool) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(version);
    let bin_dir = crate::system::version_bin_dir(&version_dir);

    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config_path = Path::new(&shell_config);
    // Backup MUST succeed before we touch the user's shell config. The
    // previous `.ok()` silently dropped backup failures, and combined with
    // `read_to_string(...).unwrap_or_default()` below could destroy the
    // file: if both backup and read failed, we'd write a fresh config over
    // an unreadable-but-still-present original, losing the user's existing
    // rc content with no recovery copy.
    backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let shell_type = detect_shell_type(&shell_config);

    // Emit shell-native env setup so the lines actually take effect in the
    // target shell (POSIX `export` vs PowerShell `$env:`). Both forms are
    // recognised by `strip_nvm_lines` for later cleanup.
    let (nvm_export, node_export) = if shell_type == "powershell" {
        (
            format!(r#"$env:NVM_HOME = "{}""#, nvm_dir.display()),
            format!(
                r#"$env:PATH = "{};{};" + $env:PATH"#,
                nvm_dir.join("shims").display(),
                bin_dir.display()
            ),
        )
    } else {
        (
            format!(r#"export NVM_HOME="{}""#, nvm_dir.display()),
            format!(
                r#"export PATH="{}:{}:$PATH""#,
                nvm_dir.join("shims").display(),
                bin_dir.display()
            ),
        )
    };

    // Read the existing config. A missing file is fine (first-time setup,
    // we'll create it), but a present file that fails to read must abort —
    // otherwise we'd overwrite content we couldn't see, with no safe way
    // back. The previous `unwrap_or_default()` collapsed both cases into
    // an empty string and proceeded to overwrite.
    //
    // Read directly and map NotFound → empty string instead of `exists()` +
    // `read_to_string`: the two-step form is a TOCTOU race (another process
    // could remove the rc file between the stat and the open), and a single
    // read is one syscall instead of two.
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };

    let nvm_dir_str = nvm_dir.display().to_string();
    let stripped = strip_nvm_lines(&content, &nvm_dir_str);

    let mut new_config = format!(
        "{}\n# NVM Rust\n{}\n{}\n",
        stripped, nvm_export, node_export
    );

    if use_on_cd {
        new_config.push_str(&cd_hook_code(shell_type));
    }

    // Atomic write (tempfile + rename): a crash mid-write on .bashrc/.zshrc
    // would corrupt the user's shell config. backup_file is a best-effort
    // safety net, but atomic_write prevents the corruption in the first
    // place and keeps this path consistent with config.json/alias.json saves.
    // On Windows the PowerShell profile directory (`Documents\PowerShell\`)
    // may not exist yet on a fresh install; atomic_write's temp file lives in
    // the parent dir, so create it first or the write fails with ENOENT.
    //
    // Propagate the create_dir_all error instead of `.ok()`-ing it: a
    // permission denial or read-only filesystem here would otherwise surface
    // as a confusing "cannot update shell config" from the atomic_write
    // below, hiding the real cause (the parent dir could not be created).
    if let Some(parent) = config_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("{}: {}", T("cannot_create_dir"), parent.display()))?;
        }
    }
    atomic_write(config_path, &new_config).context(T("cannot_update_shell_config"))?;

    Ok(())
}

pub fn remove_from_shell_config() -> Result<()> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config_path = Path::new(&shell_config);

    let nvm_dir_str = get_nvm_dir().display().to_string();

    // Read directly and map NotFound → "nothing to clean" instead of
    // `exists()` + `read_to_string`: the two-step form is a TOCTOU race
    // (another process could remove the rc file between the stat and the
    // read), and a single read is one syscall instead of two.
    //
    // The previous `if let Ok(...) = read_to_string` silently returned
    // Ok(()) on read failure, masking permission/IO errors as "nothing
    // to remove" — the user's config would remain polluted with stale
    // NVM lines and they'd never know. Surface the read error instead.
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };
    // Backup MUST succeed before we overwrite, same rationale as
    // update_shell_config. At this point the file is known to exist (read
    // succeeded), so backup_file will actually copy it.
    backup_file(config_path).context(T("shell_config_backup_failed"))?;
    let stripped = strip_nvm_lines(&content, &nvm_dir_str);
    atomic_write(config_path, &stripped)?;
    println!("{}", crate::i18n::T("shell_config_removed").green());
    Ok(())
}

/// Rewrite the shell rc file from version-specific PATH format
/// (e.g. `export PATH="shims:v24.18.0/bin:$PATH"`) to the fixed
/// `active/bin` format (`export PATH="shims:active/bin:$PATH"`).
/// This is the core of the Full Shim mode migration.
///
/// Takes `nvm_dir` as a parameter so callers that already resolved the
/// path can pass it through directly, instead of this function re-deriving
/// it from env (which broke test isolation — the env `NVM_DIR` could point
/// at a temp dir while the caller held a specific `nvm_dir`).
pub fn migrate_rc_to_shim_mode_with_dir(nvm_dir: &Path) -> Result<()> {
    // Test-isolation guard: if nvm_dir lives under $TMPDIR (test sandbox) but
    // HOME does NOT (real user home still set), refuse to write to the real
    // ~/.zshrc / ~/.bashrc. This catches the case where setup_temp_nvm_dir()
    // sandboxed NVM_DIR but forgot to also sandbox HOME. When HOME IS also
    // under TMPDIR (properly sandboxed test), writing is safe and allowed.
    // Guarded by #[cfg(test)] so production is unaffected.
    #[cfg(test)]
    {
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            let nvm_in_tmp = nvm_dir.starts_with(&tmpdir);
            let home = std::env::var("HOME").unwrap_or_default();
            let home_in_tmp = std::path::Path::new(&home).starts_with(&tmpdir);
            if nvm_in_tmp && !home_in_tmp {
                eprintln!(
                    "  ⚠ Refusing to write shell config: nvm_dir in TMPDIR but HOME not sandboxed (test isolation?)"
                );
                return Ok(());
            }
        }
    }

    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()), // No rc file, nothing to migrate
    };

    let config_path = Path::new(&shell_config);
    let nvm_dir_str = nvm_dir.display().to_string();

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };

    backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let stripped = strip_nvm_lines(&content, &nvm_dir_str);

    let shell_type = detect_shell_type(&shell_config);
    let (nvm_export, path_export, source_line) = if shell_type == "powershell" {
        (
            format!(r#"$env:NVM_HOME = "{}""#, nvm_dir_str),
            r#"$env:PATH = "$env:NVM_HOME\shims;$env:NVM_HOME\active;$env:NVM_HOME\bin;" + $env:PATH"#
                .to_string(),
            r#"Import-Module "$env:NVM_HOME\shell\nvm.psm1""#.to_string(),
        )
    } else {
        (
            format!(r#"export NVM_HOME="{}""#, nvm_dir_str),
            r#"export PATH="$NVM_HOME/shims:$NVM_HOME/active/bin:$NVM_HOME/bin:$PATH""#.to_string(),
            r#"[ -f "$NVM_HOME/bin/nvm.sh" ] && source "$NVM_HOME/bin/nvm.sh""#.to_string(),
        )
    };

    let new_config = format!(
        "{}\n# NVM Rust\n{}\n{}\n{}\n",
        stripped, nvm_export, path_export, source_line
    );

    crate::utils::atomic_write(config_path, &new_config)?;
    Ok(())
}

/// Backward-compat wrapper: re-derives nvm_dir from env and delegates to
/// [`migrate_rc_to_shim_mode_with_dir`]. Prefer the `_with_dir` variant in
/// call sites that already hold a resolved `nvm_dir`.
#[allow(dead_code)]
pub fn migrate_rc_to_shim_mode() -> Result<()> {
    migrate_rc_to_shim_mode_with_dir(&get_nvm_dir())
}

/// Check if the rc file contains version-specific PATH (old format)
/// rather than the fixed `active/bin` (new/shim format).
/// Returns true if migration is needed.
pub fn rc_has_version_specific_path() -> Result<bool> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(false),
    };

    let content = match fs::read_to_string(&shell_config) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Ok(false), // Can't read — can't determine, skip migration
    };
    let has_nvm_path =
        content.contains("nvm.rust") && content.contains("shims") && content.contains("bin");
    // Check for "active" in a PATH context — not just "active" which could
    // appear in unrelated comments, variable names, or aliases.
    // Unix uses active/bin, PowerShell uses active; (semicolon separator).
    let has_active = content.contains("active/bin")
        || content.contains("active\\bin")
        || content.contains("active;")
        || content.contains("active\"");
    Ok(has_nvm_path && !has_active)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell_config() {
        // Should return Some path even in test environment
        let result = detect_shell_config();
        assert!(result.is_some());
        // Should be a valid path string
        let path = result.unwrap();
        assert!(!path.is_empty());
    }

    #[test]
    fn test_strip_nvm_lines_removes_marker_and_exports() {
        let nvm_dir = "/home/u/.nvm.rust";
        let input = format!(
            "# my alias\n\
             alias ll='ls -l'\n\
             # NVM Rust\n\
             export NVM_HOME=\"{nvm_dir}\"\n\
             export PATH=\"/home/u/.nvm.rust/v20.0.0/bin:$PATH\"\n\
             export EDITOR=vim\n"
        );
        let out = strip_nvm_lines(&input, nvm_dir);
        // User lines survive.
        assert!(out.contains("alias ll='ls -l'"));
        assert!(out.contains("export EDITOR=vim"));
        // nvm lines are gone.
        assert!(!out.contains("NVM_HOME="));
        assert!(!out.contains("# NVM Rust"));
        assert!(!out.contains("nvm.rust"));
    }

    #[test]
    fn test_strip_nvm_lines_removes_cd_hook_all_shells() {
        let nvm_dir = "/home/u/.nvm.rust";
        // Each shell's hook block carries a distinctive marker we assert is
        // gone after stripping — `__nvm_use_on_cd` for POSIX/fish, and the
        // PowerShell prompt-wrapper symbol for powershell.
        for (st, marker) in &[
            ("bash", "__nvm_use_on_cd"),
            ("zsh", "__nvm_use_on_cd"),
            ("fish", "__nvm_use_on_cd"),
            ("powershell", "__NVM_ORIG_PROMPT"),
        ] {
            let hook = cd_hook_code(st);
            let input = format!("alias x='y'\n{hook}\nexport FOO=bar\n");
            let out = strip_nvm_lines(&input, nvm_dir);
            assert!(
                !out.contains(marker),
                "cd hook for {st} was not stripped: {out}"
            );
            assert!(out.contains("alias x='y'"));
            assert!(out.contains("export FOO=bar"));
        }
    }

    #[test]
    fn test_strip_nvm_lines_removes_powershell_env_exports() {
        // PowerShell-formatted env lines must be stripped just like POSIX
        // `export` lines, so `nvm unload` cleans up after a Windows install.
        let nvm_dir = "/home/u/.nvm.rust";
        let input = format!(
            "alias x='y'\n\
             # NVM Rust\n\
             $env:NVM_HOME = \"{nvm_dir}\"\n\
             $env:PATH = \"{nvm_dir}/v20.0.0/bin;\" + $env:PATH\n\
             export EDITOR=vim\n"
        );
        let out = strip_nvm_lines(&input, nvm_dir);
        assert!(out.contains("alias x='y'"));
        assert!(out.contains("export EDITOR=vim"));
        assert!(
            !out.contains("NVM_HOME"),
            "PowerShell NVM_HOME line kept: {out}"
        );
        assert!(
            !out.contains("$env:PATH"),
            "PowerShell PATH line kept: {out}"
        );
        assert!(!out.contains("# NVM Rust"));
    }

    #[test]
    fn test_strip_nvm_lines_preserves_unrelated_path_exports() {
        // An export PATH line that does NOT reference the nvm dir must be
        // kept — the filter must not be overzealous and drop user PATH setup.
        let nvm_dir = "/home/u/.nvm.rust";
        let input = "export PATH=/usr/local/bin:$PATH\nalias ll='ls -l'\n";
        let out = strip_nvm_lines(input, nvm_dir);
        assert!(out.contains("export PATH=/usr/local/bin:$PATH"));
    }

    #[test]
    fn test_strip_nvm_lines_idempotent() {
        // Stripping an already-clean body is a no-op (modulo trailing newline
        // joining), so re-running remove_from_shell_config is safe.
        let nvm_dir = "/home/u/.nvm.rust";
        let clean = "alias ll='ls -l'\nexport EDITOR=vim\n";
        let out = strip_nvm_lines(clean, nvm_dir);
        assert_eq!(out, clean);
    }

    // Tests that mutate NVM_DIR (and HOME) acquire the process-global
    // `ENV_TESTS_MUTEX`, serializing them against NVM_DIR-mutating tests in
    // download.rs / proxy.rs / utils.rs. The previous per-module
    // `SHELL_CFG_TESTS_MUTEX` only serialized within config.rs.
    use crate::system::ENV_TESTS_MUTEX;

    #[test]
    fn update_shell_config_surfaces_create_dir_all_failure() {
        let _guard = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Set up a HOME whose `.bashrc` parent cannot be created: place a
        // regular file where the parent directory would go. `create_dir_all`
        // then fails with NotADirectory (ENOTDIR), which must propagate
        // instead of being swallowed by the old `.ok()`.
        let tmp = tempfile::TempDir::new().expect("tempdir for HOME blocker");
        let blocker = tmp.path().join("blocker_file");
        std::fs::write(&blocker, b"not a directory").expect("write blocker file");

        let nvm_tmp = tempfile::TempDir::new().expect("tempdir for NVM_DIR");

        let old_home = std::env::var_os("HOME");
        let old_nvm_dir = std::env::var_os("NVM_DIR");
        std::env::set_var("HOME", &blocker);
        std::env::set_var("NVM_DIR", nvm_tmp.path());

        // Restore env even if assertions fail — leaking HOME=<blocker> would
        // break every subsequent test that touches the shell config.
        struct EnvGuard {
            old_home: Option<std::ffi::OsString>,
            old_nvm_dir: Option<std::ffi::OsString>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.old_home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match &self.old_nvm_dir {
                    Some(v) => std::env::set_var("NVM_DIR", v),
                    None => std::env::remove_var("NVM_DIR"),
                }
            }
        }
        let _guard_env = EnvGuard {
            old_home,
            old_nvm_dir,
        };

        let result = update_shell_config("v20.0.0", false);

        let err = result
            .expect_err("update_shell_config should fail when the parent dir cannot be created");
        let msg = format!("{err:#}");
        // The error must mention directory creation (the real cause) rather
        // than the downstream "cannot update shell config" that the old
        // `.ok()` path would have produced instead.
        assert!(
            msg.to_lowercase().contains("directory") || msg.to_lowercase().contains("create"),
            "expected create_dir_all error context, got: {msg}"
        );
    }

    #[test]
    fn migrate_rc_to_shim_mode_includes_nvm_bin_and_source_line() {
        // P0-1: After migration, PATH must include ~/.nvm.rust/bin
        // P0-2: After migration, rc must include a source nvm.sh line
        let _guard = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().expect("tempdir for HOME");
        let nvm_tmp = tempfile::TempDir::new().expect("tempdir for NVM_DIR");
        let old_home = std::env::var_os("HOME");
        let old_nvm_dir = std::env::var_os("NVM_DIR");
        let old_userprofile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", home.path());
        if cfg!(windows) {
            std::env::set_var("USERPROFILE", home.path());
        }
        std::env::set_var("NVM_DIR", nvm_tmp.path());
        struct Guard {
            old_home: Option<std::ffi::OsString>,
            old_nvm_dir: Option<std::ffi::OsString>,
            old_userprofile: Option<std::ffi::OsString>,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                match &self.old_home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match &self.old_nvm_dir {
                    Some(v) => std::env::set_var("NVM_DIR", v),
                    None => std::env::remove_var("NVM_DIR"),
                }
                if cfg!(windows) {
                    if let Some(v) = &self.old_userprofile {
                        std::env::set_var("USERPROFILE", v);
                    }
                }
            }
        }
        let _g = Guard {
            old_home,
            old_nvm_dir,
            old_userprofile,
        };
        // Find what rc file detect_shell_config will look for
        let rc_path = detect_shell_config().expect("detect_shell_config should find a path");
        let rc = std::path::Path::new(&rc_path);
        if let Some(parent) = rc.parent() {
            std::fs::create_dir_all(parent).expect("create rc parent");
        }
        // Write old-format nvm lines
        let nvm_dir_str = nvm_tmp.path().display().to_string();
        let old_content = format!(
            r#"alias ll='ls -l'
# NVM Rust
export NVM_HOME="{nvm}"
export PATH="{nvm}/shims:{nvm}/v22.0.0/bin:$PATH"
"#,
            nvm = nvm_dir_str
        );
        std::fs::write(rc, &old_content).expect("write rc");
        migrate_rc_to_shim_mode().expect("migrate should succeed");
        let content = std::fs::read_to_string(rc).expect("read rc");
        // P0-1: nvm_bin must be in PATH (literal path or $NVM_HOME variable)
        assert!(
            content.contains(&format!("{nvm_dir_str}/bin"))
                || content.contains(&format!("{}\\bin", nvm_dir_str))
                || content.contains("$NVM_HOME/bin")
                || content.contains("$env:NVM_HOME\\bin"),
            "migrated rc must include nvm/bin in PATH: {content}"
        );
        // P0-1: active must be in PATH
        assert!(
            content.contains("active"),
            "migrated rc must include active in PATH: {content}"
        );
        // P0-2: source/Import-Module reference must be present, with [ -f ] guard
        // (prevents .zshrc error when nvm.sh doesn't exist yet)
        assert!(
            content.contains("nvm.sh") || content.contains("nvm.psm1"),
            "migrated rc must include nvm.sh or nvm.psm1 reference: {content}"
        );
        // Unix source line must have [ -f ] guard (Issue 5 fix)
        if !cfg!(windows) {
            assert!(
                content.contains("[ -f ") || content.contains("Import-Module"),
                "rc source line must have [ -f ] guard (Unix) or Import-Module (Windows): {content}"
            );
        }
        // Old version-specific path must be gone
        assert!(
            !content.contains("v22.0.0/bin"),
            "migrated rc must NOT contain version-specific path: {content}"
        );
    }
}
