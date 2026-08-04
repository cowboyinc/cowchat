use clap::{Parser, Subcommand};
use cowchat_codex::app_server::CodexAppServerClient;
use cowchat_codex::config::BridgeConfig;
use cowchat_codex::mcp::{serve_stdio, WakeMcpServer};
use cowchat_codex::service::{CowchatBackend, WakeService};
use cowchat_codex::store::WakeStore;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(
    name = "cowchat-codex",
    version,
    about = "Durable Cowchat wake tools for Codex tasks"
)]
struct Cli {
    /// Bridge configuration JSON.
    #[arg(long, global = true, default_value_os_t = default_config_path())]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the wake tools over MCP stdio.
    Mcp,
    /// Validate configuration, local credentials, and the state database.
    Doctor,
    /// Print an example configuration to stdout.
    ConfigExample,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();
    if matches!(cli.command, Command::ConfigExample) {
        println!(
            "{}",
            serde_json::to_string_pretty(&BridgeConfig::example())?
        );
        return Ok(());
    }

    let config = BridgeConfig::load(&cli.config)?;
    let store = Arc::new(WakeStore::open(&config.state_db)?);
    if matches!(cli.command, Command::Doctor) {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "config": cli.config,
                "state_db": config.state_db,
                "targets": config.targets.keys().collect::<Vec<_>>(),
                "note": "doctor validates local configuration; local Cowchat transports are keyless and it does not wake a task"
            }))?
        );
        return Ok(());
    }

    let chat = Arc::new(CowchatBackend::new(config.cowchat.clone()));
    let codex = Arc::new(CodexAppServerClient::new(config.codex.clone()));
    let service = WakeService::new(config, store, chat, codex);
    serve_stdio(WakeMcpServer::new(service)).await?;
    Ok(())
}

fn default_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat/codex-wake.json"))
        .unwrap_or_else(|| PathBuf::from(".cowchat/codex-wake.json"))
}
