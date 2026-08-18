//! Proxy management commands (`nvm proxy`).
//!
//! Toggles the nvm-level proxy flag, tests connectivity through the system
//! proxy, and prints a status table with credential redaction.

use anyhow::Result;
use colored::Colorize;

use crate::i18n::{format_t, T};
use crate::utils::pad_right;

/// Mask the `user:pass@` userinfo in a proxy URL before printing it.
///
/// Proxy URLs commonly embed credentials (`http://user:pass@host:port`).
/// Printing such a URL to stdout leaks the password into terminal scrollback,
/// CI logs, and screen recordings. We replace the userinfo segment with
/// `***@`, preserving the scheme/host/port for diagnostics while hiding the
/// secret. URLs without userinfo are returned unchanged.
fn redact_proxy_credentials(url: &str) -> String {
    // Match `scheme://[userinfo@]host[:port]/path?query#frag`. userinfo ends
    // at the LAST `@` before the first `/`/`?`/`#` (authority terminator).
    // Using the last `@` handles passwords that themselves contain `@`
    // (`http://user:p@ss@host` -> userinfo=`user:p@ss`). Restricting to the
    // authority segment avoids mis-treating `@` in path/query as userinfo
    // (`http://host/path@evil` must NOT be redacted).
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        // Find end of authority: first of `/`, `?`, `#`, or end of string.
        let auth_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..auth_end];
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if !userinfo.is_empty() {
                let rest_after_userinfo = &after_scheme[at..]; // includes '@'
                return format!("{}{}{}", &url[..scheme_end + 3], "***", rest_after_userinfo);
            }
        }
    }
    url.to_string()
}

pub fn cmd_proxy(action: Option<&str>) -> Result<()> {
    use crate::proxy::{get_system_proxy, proxy_status, set_proxy_enabled, test_connectivity};

    match action {
        Some("on") => {
            let sys_proxy = get_system_proxy();
            if sys_proxy.is_none() {
                println!();
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("proxy_no_system_proxy").yellow()
                );
                println!("  {} {}", "→".dimmed(), T("proxy_set_env_vars").dimmed());
                println!();
                return Ok(());
            }

            // Enable proxy first, so the connectivity test routes through it.
            set_proxy_enabled(true)?;

            // Test connectivity via the now-enabled proxy.
            println!("  {} {}", "›".dimmed(), T("testing_connectivity"));
            let (baidu_ok, google_ok) = test_connectivity();

            if baidu_ok || google_ok {
                println!();
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("proxy_enabled").green(),
                    T("proxy_will_be_used").green()
                );
                print!("    ");
                if baidu_ok {
                    print!("{}  ", T("proxy_test_baidu_ok").green());
                } else {
                    print!("{}  ", T("proxy_test_baidu_fail").red());
                }
                if google_ok {
                    println!("{}", T("proxy_test_google_ok").green());
                } else {
                    println!("{}", T("proxy_test_google_fail").red());
                }
                println!();
            } else {
                // Proxy did not work; roll back so downloads do not hang.
                set_proxy_enabled(false)?;
                println!();
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("neither_reachable").yellow()
                );
                println!("  {} {}", "→".dimmed(), T("check_proxy_settings").dimmed());
                println!();
            }
        }
        Some("off") => {
            set_proxy_enabled(false)?;
            println!();
            println!("  {} {}", "✓".green().bold(), T("proxy_disabled").green());
            println!();
        }
        Some(other) => {
            anyhow::bail!("{}", format_t("unknown_action", &[other.to_string()]));
        }
        None => {
            let status = proxy_status();
            let sys_proxy = status.system_proxy.clone();

            println!();
            println!("  {}", T("proxy_status_title").cyan().bold());
            println!();

            // NVM proxy toggle. `pad_right` correctly handles ANSI-coloured
            // labels (it strips escape codes when measuring width), so the
            // two rows line up regardless of which color the label uses.
            const STATUS_COL: usize = 10;
            let nvm_state = if status.nvm_proxy_enabled {
                T("proxy_state_on").green().bold().to_string()
            } else {
                T("proxy_state_off").red().bold().to_string()
            };
            println!(
                "    {} {}",
                pad_right(&"nvm:".dimmed().to_string(), STATUS_COL),
                nvm_state
            );

            // System proxy env. Redact embedded credentials before printing:
            // `HTTPS_PROXY=http://user:pass@proxy:8080` is a common pattern,
            // and printing the raw URL to stdout would leak the password into
            // terminal scrollback, CI logs, and screen recordings.
            let sys_state = match &sys_proxy {
                Some(p) => format!("{}", redact_proxy_credentials(p).dimmed()),
                None => T("not_set").red().to_string(),
            };
            println!(
                "    {} {}",
                pad_right(&"system:".dimmed().to_string(), STATUS_COL),
                sys_state
            );

            println!();

            if status.nvm_proxy_enabled {
                if sys_proxy.is_some() {
                    println!("  {} {}", "✓".green().bold(), T("proxy_active").green());
                } else {
                    println!(
                        "  {} {}",
                        "⚠".yellow().bold(),
                        T("proxy_on_no_env").yellow()
                    );
                }
            } else {
                println!("  {} {}", "ℹ".cyan().bold(), T("proxy_off_direct").cyan());
            }

            println!();
            println!(
                "  {} {}",
                T("usage_label").dimmed(),
                T("proxy_usage_hint").yellow().bold()
            );
            println!();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_proxy_credentials_masks_userinfo() {
        // user:pass@ must be replaced with ***@
        assert_eq!(
            redact_proxy_credentials("http://user:pass@proxy.corp:8080"),
            "http://***@proxy.corp:8080"
        );
        assert_eq!(
            redact_proxy_credentials("https://bob:s3cr3t@10.0.0.1:3128"),
            "https://***@10.0.0.1:3128"
        );
        // User-only (no password) is still masked.
        assert_eq!(
            redact_proxy_credentials("http://user@host:80"),
            "http://***@host:80"
        );
    }

    #[test]
    fn test_redact_proxy_credentials_preserves_no_creds() {
        // No userinfo -> returned unchanged.
        assert_eq!(
            redact_proxy_credentials("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            redact_proxy_credentials("socks5://localhost:1080"),
            "socks5://localhost:1080"
        );
        // Not a URL at all.
        assert_eq!(redact_proxy_credentials("not set"), "not set");
    }
}
