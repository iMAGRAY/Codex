use chrono::DateTime;
use chrono::Utc;
use codex_common::CliConfigOverrides;
use codex_core::CodexAuth;
use codex_core::auth::AccountPoolSummary;
use codex_core::auth::CLIENT_ID;
use codex_core::auth::login_with_api_key;
use codex_core::auth::logout;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_login::ServerOptions;
use codex_login::run_login_server;
use codex_protocol::mcp_protocol::AuthMode;
use std::path::PathBuf;

pub async fn login_with_chatgpt(codex_home: PathBuf) -> std::io::Result<()> {
    let opts = ServerOptions::new(codex_home, CLIENT_ID.to_string());
    let server = run_login_server(opts)?;

    eprintln!(
        "Starting local login server on http://localhost:{}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{}",
        server.actual_port, server.auth_url,
    );

    server.block_until_done().await
}

pub async fn run_login_with_chatgpt(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match login_with_chatgpt(config.codex_home).await {
        Ok(_) => {
            eprintln!("Successfully logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match login_with_api_key(&config.codex_home, &api_key) {
        Ok(_) => {
            eprintln!("Successfully logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_status(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match CodexAuth::from_codex_home(&config.codex_home) {
        Ok(Some(auth)) => match auth.mode {
            AuthMode::ApiKey => match auth.get_token().await {
                Ok(api_key) => {
                    eprintln!("Logged in using an API key - {}", safe_format_key(&api_key));
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Unexpected error retrieving API key: {e}");
                    std::process::exit(1);
                }
            },
            AuthMode::ChatGPT => {
                let summary = auth.account_pool_summary();
                emit_account_status(&summary);

                std::process::exit(0);
            }
        },
        Ok(None) => {
            eprintln!("Not logged in");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_logout(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match logout(&config.codex_home) {
        Ok(true) => {
            eprintln!("Successfully logged out");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    let config_overrides = ConfigOverrides::default();
    match Config::load_with_cli_overrides(cli_overrides, config_overrides) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

fn format_ago(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() <= 0 {
        return "just now".to_string();
    }
    let secs = delta.num_seconds();
    if secs < 60 {
        return format!("{}s", secs);
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        return format!("{}m", mins);
    }
    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{}h", hours);
    }
    let days = delta.num_days();
    if days < 7 {
        return format!("{}d", days);
    }
    format!("{}w", days / 7)
}

fn format_eta(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    if ts <= now {
        return "0s".to_string();
    }
    let delta = ts - now;
    let secs = delta.num_seconds();
    if secs < 60 {
        return format!("{}s", secs);
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        let rem = secs - mins * 60;
        if rem == 0 {
            return format!("{}m", mins);
        }
        return format!("{}m{}s", mins, rem);
    }
    let hours = delta.num_hours();
    if hours < 24 {
        let rem = mins - hours * 60;
        if rem == 0 {
            return format!("{}h", hours);
        }
        return format!("{}h {}m", hours, rem);
    }
    let days = delta.num_days();
    if days < 7 {
        let rem = hours - days * 24;
        if rem == 0 {
            return format!("{}d", days);
        }
        return format!("{}d {}h", days, rem);
    }
    let weeks = days / 7;
    let rem = days % 7;
    if rem == 0 {
        return format!("{}w", weeks);
    }
    format!("{}w {}d", weeks, rem)
}

fn emit_account_status(summary: &AccountPoolSummary) {
    if summary.total_accounts <= 1 {
        eprintln!("Logged in using ChatGPT");
        return;
    }

    let active_display = summary
        .active_index
        .map(|idx| format!("{}/{}", idx + 1, summary.total_accounts))
        .unwrap_or_else(|| format!("?/{}", summary.total_accounts));
    let rotation = if summary.rotation_enabled {
        "enabled"
    } else {
        "disabled"
    };
    eprintln!(
        "Logged in using ChatGPT — account {} (rotation {}).",
        active_display, rotation
    );

    let standby = summary.available_accounts.saturating_sub(1);
    eprintln!(
        "Available: {} | Cooldown: {} | Inactive: {}",
        standby, summary.cooldown_accounts, summary.inactive_accounts
    );

    if summary.cooldown_accounts > 0 {
        if let Some(next_at) = summary.next_available_at {
            eprintln!(
                "Next account unlock in {} ({} waiting).",
                format_eta(next_at),
                summary.cooldown_accounts
            );
        } else {
            eprintln!(
                "{} account(s) in cooldown; automatic rotation will resume on reset.",
                summary.cooldown_accounts
            );
        }
    } else if standby > 0 && summary.rotation_enabled {
        eprintln!("Rotation standby: {} spare account(s).", standby);
    }

    if let Some(last_switch) = summary.last_rotation_at {
        eprintln!("Last automatic switch {} ago.", format_ago(last_switch));
    }

    if let Some(last_limit) = summary.last_rate_limit_at {
        eprintln!("Last rate-limit signal {} ago.", format_ago(last_limit));
    }
}

#[cfg(test)]
mod tests {
    use super::safe_format_key;

    #[test]
    fn formats_long_key() {
        let key = "sk-proj-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "sk-proj-***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "sk-proj-12345";
        assert_eq!(safe_format_key(key), "***");
    }
}
