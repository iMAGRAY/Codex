use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use clap::ArgGroup;
use codex_cli::mcp::cli::WizardArgs;
use codex_cli::mcp::wizard::WizardOutcome;
use codex_cli::mcp::wizard::build_non_interactive as build_wizard_non_interactive;
use codex_cli::mcp::wizard::confirm_apply as wizard_confirm_apply;
use codex_cli::mcp::wizard::render_json_summary as wizard_render_json;
use codex_cli::mcp::wizard::run_interactive as run_wizard_interactive;
use codex_common::CliConfigOverrides;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::config::find_codex_home;
use codex_core::config::load_global_mcp_servers;
use codex_core::config::migrations::mcp::MigrationOptions;
use codex_core::config::migrations::mcp::{self};
use codex_core::config::write_global_mcp_servers;
use codex_core::config_types::McpServerConfig;
use codex_core::config_types::McpServerTransportConfig;
use codex_core::features::Feature;
use codex_core::mcp::auth::compute_auth_statuses;
use codex_core::mcp::registry::McpRegistry;
use codex_core::mcp::registry::validate_server_name;
use codex_core::mcp::templates::TemplateCatalog;
use codex_core::protocol::McpAuthStatus;
use codex_rmcp_client::delete_oauth_tokens;
use codex_rmcp_client::perform_oauth_login;

#[derive(Debug, clap::Parser)]
pub struct McpCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[command(subcommand)]
    pub subcommand: Option<McpSubcommand>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, clap::Subcommand)]
pub enum McpSubcommand {
    /// [experimental] Run the Codex MCP server (stdio transport).
    Serve,

    /// [experimental] List configured MCP servers.
    List(ListArgs),

    /// [experimental] Show details for a configured MCP server.
    Get(GetArgs),

    /// [experimental] Add a global MCP server entry.
    Add(AddArgs),

    /// [experimental] Remove a global MCP server entry.
    Remove(RemoveArgs),

    /// [experimental] Authenticate with a configured MCP server via OAuth.
    /// Requires experimental_use_rmcp_client = true in config.toml.
    Login(LoginArgs),

    /// [experimental] Remove stored OAuth credentials for a server.
    /// Requires experimental_use_rmcp_client = true in config.toml.
    Logout(LogoutArgs),

    /// [experimental] Migrate MCP configuration to the latest schema.
    Migrate(MigrateArgs),

    /// [experimental] Launch the MCP configuration wizard (preview).
    Wizard(WizardArgs),
}

#[derive(Debug, clap::Parser)]
pub struct ListArgs {
    /// Output the configured servers as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Parser)]
pub struct GetArgs {
    /// Name of the MCP server to display.
    pub name: String,

    /// Output the server configuration as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Parser)]
pub struct AddArgs {
    /// Name for the MCP server configuration.
    pub name: String,

    #[command(flatten)]
    pub transport_args: AddMcpTransportArgs,
}

#[derive(Debug, clap::Args)]
#[command(
    group(
        ArgGroup::new("transport")
            .args(["command", "url"])
            .required(true)
            .multiple(false)
    )
)]
pub struct AddMcpTransportArgs {
    #[command(flatten)]
    pub stdio: Option<AddMcpStdioArgs>,

    #[command(flatten)]
    pub streamable_http: Option<AddMcpStreamableHttpArgs>,
}

#[derive(Debug, clap::Args)]
pub struct AddMcpStdioArgs {
    /// Command to launch the MCP server.
    /// Use --url for a streamable HTTP server.
    #[arg(trailing_var_arg = true, num_args = 0..)]
    pub command: Vec<String>,

    /// Environment variables to set when launching the server.
    /// Only valid with stdio servers.
    #[arg(long, value_parser = parse_env_pair, value_name = "KEY=VALUE")]
    pub env: Vec<(String, String)>,
}

#[derive(Debug, clap::Args)]
pub struct AddMcpStreamableHttpArgs {
    /// URL for a streamable HTTP MCP server.
    #[arg(long)]
    pub url: String,

    /// Optional environment variable to read for a bearer token.
    #[arg(
        long = "bearer-token-env-var",
        value_name = "ENV_VAR",
        requires = "url"
    )]
    pub bearer_token_env_var: Option<String>,
}

#[derive(Debug, clap::Parser)]
pub struct RemoveArgs {
    /// Name of the MCP server configuration to remove.
    pub name: String,
}

#[derive(Debug, clap::Parser)]
pub struct LoginArgs {
    /// Name of the MCP server to authenticate with oauth.
    pub name: String,
}

#[derive(Debug, clap::Parser)]
pub struct LogoutArgs {
    /// Name of the MCP server to deauthenticate.
    pub name: String,
}

#[derive(Debug, clap::Parser)]
pub struct MigrateArgs {
    /// Apply migration changes instead of performing a dry-run preview.
    #[arg(long, default_value_t = false)]
    pub apply: bool,

    /// Migrate even when the schema version is already up-to-date.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

impl McpCli {
    pub async fn run(self, codex_linux_sandbox_exe: Option<PathBuf>) -> Result<()> {
        let McpCli {
            config_overrides,
            subcommand,
        } = self;
        let subcommand = subcommand.unwrap_or(McpSubcommand::Serve);

        match subcommand {
            McpSubcommand::Serve => {
                codex_mcp_server::run_main(codex_linux_sandbox_exe, config_overrides).await?;
            }
            McpSubcommand::List(args) => {
                run_list(&config_overrides, args).await?;
            }
            McpSubcommand::Get(args) => {
                run_get(&config_overrides, args).await?;
            }
            McpSubcommand::Add(args) => {
                run_add(&config_overrides, args).await?;
            }
            McpSubcommand::Remove(args) => {
                run_remove(&config_overrides, args).await?;
            }
            McpSubcommand::Login(args) => {
                run_login(&config_overrides, args).await?;
            }
            McpSubcommand::Logout(args) => {
                run_logout(&config_overrides, args).await?;
            }
            McpSubcommand::Migrate(args) => {
                run_migrate(&config_overrides, args).await?;
            }
            McpSubcommand::Wizard(args) => {
                run_wizard(&config_overrides, args).await?;
            }
        }

        Ok(())
    }
}

async fn run_add(config_overrides: &CliConfigOverrides, add_args: AddArgs) -> Result<()> {
    config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;

    let AddArgs {
        name,
        transport_args,
    } = add_args;

    validate_server_name(&name)?;

    let codex_home = find_codex_home().context("failed to resolve CODEX_HOME")?;
    let mut servers = load_global_mcp_servers(&codex_home)
        .await
        .with_context(|| format!("failed to load MCP servers from {}", codex_home.display()))?;

    let transport = match transport_args {
        AddMcpTransportArgs {
            stdio: Some(stdio), ..
        } => {
            let mut command_parts = stdio.command.into_iter();
            let command_bin = command_parts
                .next()
                .ok_or_else(|| anyhow!("command is required"))?;
            let command_args: Vec<String> = command_parts.collect();
            let env_map = if stdio.env.is_empty() {
                None
            } else {
                Some(stdio.env.into_iter().collect::<HashMap<_, _>>())
            };
            McpServerTransportConfig::Stdio {
                command: command_bin,
                args: command_args,
                env: env_map,
            }
        }
        AddMcpTransportArgs {
            streamable_http: Some(streamable_http),
            ..
        } => McpServerTransportConfig::StreamableHttp {
            url: streamable_http.url,
            bearer_token_env_var: streamable_http.bearer_token_env_var,
        },
        AddMcpTransportArgs { .. } => bail!("exactly one of --command or --url must be provided"),
    };

    let mut new_entry = McpServerConfig::default();
    new_entry.transport = transport;

    servers.insert(name.clone(), new_entry);

    write_global_mcp_servers(&codex_home, &servers)
        .with_context(|| format!("failed to write MCP servers to {}", codex_home.display()))?;

    println!("Added global MCP server '{name}'.");

    Ok(())
}

async fn run_remove(config_overrides: &CliConfigOverrides, remove_args: RemoveArgs) -> Result<()> {
    config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;

    let RemoveArgs { name } = remove_args;

    validate_server_name(&name)?;

    let codex_home = find_codex_home().context("failed to resolve CODEX_HOME")?;
    let mut servers = load_global_mcp_servers(&codex_home)
        .await
        .with_context(|| format!("failed to load MCP servers from {}", codex_home.display()))?;

    let removed = servers.remove(&name).is_some();

    if removed {
        write_global_mcp_servers(&codex_home, &servers)
            .with_context(|| format!("failed to write MCP servers to {}", codex_home.display()))?;
        println!("Removed global MCP server '{name}'.");
    } else {
        println!("No MCP server named '{name}' found.");
    }

    Ok(())
}

async fn run_login(config_overrides: &CliConfigOverrides, login_args: LoginArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    if !config.features.enabled(Feature::RmcpClient) {
        bail!(
            "OAuth login is only supported when experimental_use_rmcp_client is true in config.toml."
        );
    }

    let LoginArgs { name } = login_args;

    let Some(server) = config.mcp_servers.get(&name) else {
        bail!("No MCP server named '{name}' found.");
    };

    let url = match &server.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => url.clone(),
        _ => bail!("OAuth login is only supported for streamable HTTP servers."),
    };

    perform_oauth_login(&name, &url, config.mcp_oauth_credentials_store_mode).await?;
    println!("Successfully logged in to MCP server '{name}'.");
    Ok(())
}

async fn run_logout(config_overrides: &CliConfigOverrides, logout_args: LogoutArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    let LogoutArgs { name } = logout_args;

    let server = config
        .mcp_servers
        .get(&name)
        .ok_or_else(|| anyhow!("No MCP server named '{name}' found in configuration."))?;

    let url = match &server.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => url.clone(),
        _ => bail!("OAuth logout is only supported for streamable_http transports."),
    };

    match delete_oauth_tokens(&name, &url, config.mcp_oauth_credentials_store_mode) {
        Ok(true) => println!("Removed OAuth credentials for '{name}'."),
        Ok(false) => println!("No OAuth credentials stored for '{name}'."),
        Err(err) => return Err(anyhow!("failed to delete OAuth credentials: {err}")),
    }

    Ok(())
}

async fn run_migrate(config_overrides: &CliConfigOverrides, args: MigrateArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    if !config.experimental_mcp_overhaul && !args.force {
        bail!(
            "MCP overhaul features are disabled. Enable `experimental.mcp_overhaul=true` or rerun with --force."
        );
    }

    let options = MigrationOptions {
        dry_run: !args.apply,
        force: args.force,
    };

    let report = mcp::migrate_to_v2(&config.codex_home, &options).with_context(|| {
        format!(
            "failed to migrate configuration at {}",
            config.codex_home.display()
        )
    })?;

    if options.dry_run {
        println!(
            "Dry run complete (from schema v{} → v{}). Changes detected: {}",
            report.from_version, report.to_version, report.changes_detected
        );
    } else {
        println!(
            "Migration applied (schema v{} → v{}). Backup created: {}",
            report.from_version, report.to_version, report.backed_up
        );
    }

    for note in report.notes {
        println!("• {note}");
    }

    Ok(())
}

async fn run_list(config_overrides: &CliConfigOverrides, list_args: ListArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    let mut entries: Vec<_> = config.mcp_servers.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));

    let auth_statuses = compute_auth_statuses(
        config.mcp_servers.iter(),
        config.mcp_oauth_credentials_store_mode,
    )
    .await;

    if list_args.json {
        let json_entries: Vec<_> = entries
            .into_iter()
            .map(|(name, cfg)| server_to_json(name, cfg, auth_statuses.get(name.as_str()).copied()))
            .collect();
        let output = serde_json::to_string_pretty(&json_entries)?;
        println!("{output}");
        return Ok(());
    }

    if entries.is_empty() {
        println!("No MCP servers configured yet. Try `codex mcp add my-tool -- my-command`.");
        return Ok(());
    }

    let mut stdio_rows: Vec<Vec<String>> = Vec::new();
    let mut http_rows: Vec<Vec<String>> = Vec::new();

    for (name, cfg) in entries {
        let display = cfg.display_name.as_deref().unwrap_or("-").to_string();
        let status = if cfg.enabled { "enabled" } else { "disabled" }.to_string();
        let auth = auth_statuses
            .get(name.as_str())
            .copied()
            .unwrap_or(McpAuthStatus::Unsupported)
            .to_string();

        match &cfg.transport {
            McpServerTransportConfig::Stdio { command, args, env } => {
                let args_display = if args.is_empty() {
                    "-".to_string()
                } else {
                    args.join(" ")
                };
                let env_display = match env.as_ref() {
                    None => "-".to_string(),
                    Some(map) if map.is_empty() => "-".to_string(),
                    Some(map) => {
                        let mut pairs: Vec<_> = map.iter().collect();
                        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                        pairs
                            .into_iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                };

                stdio_rows.push(vec![
                    name.clone(),
                    display,
                    command.clone(),
                    args_display,
                    env_display,
                    status,
                    auth,
                ]);
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token_env_var,
            } => {
                http_rows.push(vec![
                    name.clone(),
                    display,
                    url.clone(),
                    bearer_token_env_var
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    status,
                    auth,
                ]);
            }
        }
    }

    if !stdio_rows.is_empty() {
        render_table(
            &[
                "Name", "Display", "Command", "Args", "Env", "Status", "Auth",
            ],
            &stdio_rows,
        );
    }

    if !stdio_rows.is_empty() && !http_rows.is_empty() {
        println!();
    }

    if !http_rows.is_empty() {
        render_table(
            &[
                "Name",
                "Display",
                "Url",
                "Bearer Token Env Var",
                "Status",
                "Auth",
            ],
            &http_rows,
        );
    }

    Ok(())
}

async fn run_get(config_overrides: &CliConfigOverrides, get_args: GetArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    let templates = TemplateCatalog::load_default().unwrap_or_else(|err| {
        tracing::warn!("Failed to load MCP templates: {err}");
        TemplateCatalog::empty()
    });
    let registry = McpRegistry::new(&config, templates);

    let server = registry
        .server(&get_args.name)
        .ok_or_else(|| anyhow!("No MCP server named '{name}' found.", name = get_args.name))?;

    if get_args.json {
        let json_value = server_to_json(
            &get_args.name,
            server,
            None, // auth status omitted for single view
        );
        let output = serde_json::to_string_pretty(&json_value)?;
        println!("{output}");
        return Ok(());
    }

    println!("{}", get_args.name);
    println!("  enabled: {}", server.enabled);
    if let Some(display_name) = &server.display_name {
        println!("  display_name: {display_name}");
    }
    if let Some(category) = &server.category {
        println!("  category: {category}");
    }
    if let Some(template_id) = &server.template_id {
        println!("  template_id: {template_id}");
    }
    if let Some(description) = &server.description {
        println!("  description: {description}");
    }
    if !server.tags.is_empty() {
        println!("  tags: {}", server.tags.join(", "));
    }
    if let Some(created_at) = &server.created_at {
        println!("  created_at: {created_at}");
    }
    if let Some(last_verified_at) = &server.last_verified_at {
        println!("  last_verified_at: {last_verified_at}");
    }

    if let Some(metadata) = &server.metadata {
        if metadata.is_empty() {
            println!("  metadata: <empty>");
        } else {
            let mut entries: Vec<_> = metadata.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            let joined = entries
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  metadata: {joined}");
        }
    }

    match &server.transport {
        McpServerTransportConfig::Stdio { command, args, env } => {
            println!("  transport: stdio");
            println!("  command: {command}");
            let args_display = if args.is_empty() {
                "-".to_string()
            } else {
                args.join(" ")
            };
            println!("  args: {args_display}");
            match env.as_ref() {
                None => println!("  env: -"),
                Some(map) if map.is_empty() => println!("  env: -"),
                Some(map) => {
                    let mut pairs: Vec<_> = map.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    for (k, v) in pairs {
                        println!("  env: {k}={v}");
                    }
                }
            }
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
        } => {
            println!("  transport: streamable_http");
            println!("  url: {url}");
            match bearer_token_env_var {
                Some(var) => println!("  bearer_token_env_var: {var}"),
                None => println!("  bearer_token_env_var: -"),
            }
        }
    }

    if let Some(startup_timeout) = server.startup_timeout_sec {
        println!("  startup_timeout_sec: {}", startup_timeout.as_secs_f64());
    }
    if let Some(tool_timeout) = server.tool_timeout_sec {
        println!("  tool_timeout_sec: {}", tool_timeout.as_secs_f64());
    }

    if let Some(auth) = &server.auth {
        println!("  auth.type: {}", auth.kind.as_deref().unwrap_or("<unset>"));
        if let Some(secret_ref) = &auth.secret_ref {
            println!("  auth.secret_ref: {secret_ref}");
        }
        if let Some(env) = &auth.env {
            if env.is_empty() {
                println!("  auth.env: <empty>");
            } else {
                let mut pairs: Vec<_> = env.iter().collect();
                pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                let joined = pairs
                    .into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  auth.env: {joined}");
            }
        }
    }
    if let Some(health) = &server.healthcheck {
        println!(
            "  healthcheck.type: {}",
            health.kind.as_deref().unwrap_or("<unset>")
        );
        if let Some(command) = &health.command {
            println!("  healthcheck.command: {command}");
        }
        if !health.args.is_empty() {
            println!("  healthcheck.args: {}", health.args.join(" "));
        }
        if let Some(timeout_ms) = health.timeout_ms {
            println!("  healthcheck.timeout_ms: {timeout_ms}");
        }
        if let Some(interval) = health.interval_seconds {
            println!("  healthcheck.interval_seconds: {interval}");
        }
        if let Some(endpoint) = &health.endpoint {
            println!("  healthcheck.endpoint: {endpoint}");
        }
        if let Some(protocol) = &health.protocol {
            println!("  healthcheck.protocol: {protocol}");
        }
    }

    println!(
        "  experimental_overhaul_enabled: {}",
        registry.experimental_enabled()
    );
    println!("  remove: codex mcp remove {}", get_args.name);

    Ok(())
}

async fn run_wizard(config_overrides: &CliConfigOverrides, args: WizardArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .await
        .context("failed to load configuration")?;

    if !config.experimental_mcp_overhaul {
        bail!(
            "MCP overhaul features are disabled. Enable `experimental.mcp_overhaul=true` to use the wizard."
        );
    }

    let templates = TemplateCatalog::load_default().unwrap_or_else(|err| {
        tracing::warn!("Failed to load MCP templates: {err}");
        TemplateCatalog::empty()
    });
    let registry = McpRegistry::new(&config, templates.clone());

    let has_non_interactive_inputs = args.name.is_some()
        || args.command.is_some()
        || !args.args.is_empty()
        || !args.env.is_empty()
        || args.startup_timeout_ms.is_some()
        || args.tool_timeout_ms.is_some()
        || args.description.is_some()
        || !args.tags.is_empty()
        || args.auth_type.is_some()
        || args.auth_secret_ref.is_some()
        || !args.auth_env.is_empty()
        || args.health_type.is_some()
        || args.health_command.is_some()
        || !args.health_args.is_empty()
        || args.health_timeout_ms.is_some()
        || args.health_interval_seconds.is_some()
        || args.health_endpoint.is_some()
        || args.health_protocol.is_some();

    if args.json {
        if has_non_interactive_inputs {
            let outcome = build_wizard_non_interactive(&registry, &args)?;
            println!("{}", wizard_render_json(&outcome)?);
        } else {
            let summary = serde_json::json!({
                "experimental_overhaul": registry.experimental_enabled(),
                "server_count": registry.servers().count(),
                "template_ids": templates
                    .templates()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
                "preselected_template": args.template,
            });
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        return Ok(());
    }

    let outcome = if has_non_interactive_inputs {
        build_wizard_non_interactive(&registry, &args)?
    } else {
        run_wizard_interactive(&registry, args.template.as_deref())?
    };

    let mut applied = false;
    let mut summary_shown = false;

    if args.apply {
        print_wizard_summary(&outcome);
        summary_shown = true;
        registry
            .upsert_server(&outcome.name, outcome.server.clone())
            .context("failed to persist MCP server")?;
        applied = true;
    } else if wizard_confirm_apply(&outcome)? {
        summary_shown = true;
        registry
            .upsert_server(&outcome.name, outcome.server.clone())
            .context("failed to persist MCP server")?;
        applied = true;
    }

    if applied {
        println!(
            "Saved server '{name}' to {path}",
            name = outcome.name,
            path = registry.codex_home().display()
        );
        if !summary_shown {
            print_wizard_summary(&outcome);
        }
    } else {
        println!("No changes saved.");
    }

    Ok(())
}

fn print_wizard_summary(outcome: &WizardOutcome) {
    println!("Configuration summary:");
    for (key, value) in outcome.summary() {
        println!("  {key}: {value}");
    }
}

fn server_to_json(
    name: &str,
    cfg: &McpServerConfig,
    auth: Option<McpAuthStatus>,
) -> serde_json::Value {
    let transport = match &cfg.transport {
        McpServerTransportConfig::Stdio { command, args, env } => serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
            "env": env,
        }),
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
        } => serde_json::json!({
            "type": "streamable_http",
            "url": url,
            "bearer_token_env_var": bearer_token_env_var,
        }),
    };

    let (command, args_field, env_field, url_field, bearer_field) = match &cfg.transport {
        McpServerTransportConfig::Stdio { command, args, env } => {
            let env_map = env.as_ref().map(|m| {
                let mut ordered = BTreeMap::new();
                for (k, v) in m {
                    ordered.insert(k.clone(), v.clone());
                }
                ordered
            });
            (
                Some(command.clone()),
                Some(args.clone()),
                env_map,
                None,
                None,
            )
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
        } => (
            None,
            None,
            None,
            Some(url.clone()),
            bearer_token_env_var.clone(),
        ),
    };

    serde_json::json!({
        "name": name,
        "display_name": cfg.display_name,
        "category": cfg.category,
        "template_id": cfg.template_id,
        "description": cfg.description,
        "command": command,
        "args": args_field,
        "env": env_field,
        "url": url_field,
        "bearer_token_env_var": bearer_field,
        "tags": cfg.tags,
        "created_at": cfg.created_at,
        "last_verified_at": cfg.last_verified_at,
        "metadata": cfg.metadata,
        "auth": cfg.auth,
        "healthcheck": cfg.healthcheck,
        "enabled": cfg.enabled,
        "transport": transport,
        "startup_timeout_sec": cfg.startup_timeout_sec.map(|d| d.as_secs_f64()),
        "tool_timeout_sec": cfg.tool_timeout_sec.map(|d| d.as_secs_f64()),
        "auth_status": auth,
    })
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i >= widths.len() {
                widths.push(cell.len());
            } else {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let header_line = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "{h:<width$}",
                width = widths.get(i).copied().unwrap_or(h.len())
            )
        })
        .collect::<Vec<_>>()
        .join("  ");
    println!("{header_line}");

    for row in rows {
        let line = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                format!(
                    "{cell:<width$}",
                    width = widths.get(i).copied().unwrap_or(cell.len())
                )
            })
            .collect::<Vec<_>>()
            .join("  ");
        println!("{line}");
    }
}

fn parse_env_pair(raw: &str) -> Result<(String, String), String> {
    let mut parts = raw.splitn(2, '=');
    let key = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "environment entries must be in KEY=VALUE form".to_string())?;
    let value = parts
        .next()
        .map(str::to_string)
        .ok_or_else(|| "environment entries must be in KEY=VALUE form".to_string())?;

    Ok((key.to_string(), value))
}
