use clap::{Parser, Subcommand};
use cowchat_codex::app_server::CodexAppServerClient;
use cowchat_codex::config::BridgeConfig;
use cowchat_codex::mcp::{serve_stdio, WakeMcpServer};
use cowchat_codex::service::{CowchatBackend, WakeService};
use cowchat_codex::store::WakeStore;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

mod listener;

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
    /// Install Codex session hooks without replacing existing hooks.
    Install {
        /// Start the managed Codex app-server daemon after installing hooks.
        #[arg(long)]
        start_daemon: bool,
    },
    /// Wait for one Codex session's Cowchat messages and wake its thread.
    ListenSession {
        #[arg(long)]
        mapping: PathBuf,
    },
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
    if let Command::Install { start_daemon } = &cli.command {
        install_hooks(*start_daemon)?;
        return Ok(());
    }
    if let Command::ListenSession { mapping } = &cli.command {
        listener::listen(mapping).await?;
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

fn install_hooks(start_daemon: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = directories::BaseDirs::new()
        .ok_or("cannot locate home directory")?
        .home_dir()
        .to_path_buf();
    install_hooks_at(&home)?;
    if start_daemon {
        match std::process::Command::new("codex")
            .args(["app-server", "daemon", "start"])
            .output()
        {
            Ok(output) if output.status.success() => {}
            _ => eprintln!("Start the Codex app-server daemon before using Cowchat session wakes"),
        }
    } else {
        eprintln!("For session wakes, start the managed Codex app-server daemon with `codex app-server daemon start`.");
    }
    Ok(())
}

fn install_hooks_at(home: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let script_dir = home.join(".cowchat/codex-hook");
    std::fs::create_dir_all(&script_dir)?;
    let script = script_dir.join("session.py");
    std::fs::write(
        &script,
        include_str!("../../../plugins/cowchat-codex/hooks/session.py"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_dir, std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::write(
        script_dir.join("binary"),
        std::env::current_exe()?.to_string_lossy().as_bytes(),
    )?;
    let configured_path = home.join(".codex/hooks.json");
    let hooks_path = if std::fs::symlink_metadata(&configured_path).is_ok() {
        std::fs::canonicalize(&configured_path)?
    } else {
        configured_path
    };
    let existing_permissions = std::fs::metadata(&hooks_path)
        .ok()
        .map(|meta| meta.permissions());
    let mut config: serde_json::Value = if hooks_path.exists() {
        serde_json::from_slice(&std::fs::read(&hooks_path)?)?
    } else {
        json!({"hooks":{}})
    };
    let hooks = config
        .get_mut("hooks")
        .and_then(|v| v.as_object_mut())
        .ok_or("Codex hooks.json needs a hooks object")?;
    let quoted = format!("'{}'", script.display().to_string().replace('\'', "'\\''"));
    let command = format!("python3 {quoted}");
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "Stop",
        "SessionEnd",
    ] {
        let entries = hooks.entry(event).or_insert_with(|| json!([]));
        let items = entries
            .as_array_mut()
            .ok_or("Codex hook event needs an array")?;
        let already_installed = items.iter().any(|item| {
            item["hooks"]
                .as_array()
                .is_some_and(|steps| steps.iter().any(|step| step["command"] == command))
        });
        if !already_installed {
            items.push(json!({"hooks":[{"type":"command","command":command,"timeout":20}]}));
        }
    }
    std::fs::create_dir_all(hooks_path.parent().ok_or("invalid hooks path")?)?;
    let temporary = hooks_path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&config)?)?;
    if let Some(permissions) = existing_permissions {
        std::fs::set_permissions(&temporary, permissions)?;
    }
    std::fs::rename(temporary, hooks_path)?;
    println!("Cowchat Codex hooks installed");
    Ok(())
}

fn default_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat/codex-wake.json"))
        .unwrap_or_else(|| PathBuf::from(".cowchat/codex-wake.json"))
}

#[cfg(test)]
mod install_tests {
    use super::*;

    #[test]
    fn install_preserves_existing_hooks_and_is_idempotent() {
        let home =
            std::env::temp_dir().join(format!("cowchat-codex-install-{}", uuid::Uuid::new_v4()));
        let codex = home.join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        let path = codex.join("hooks.json");
        std::fs::write(&path, r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"existing-hook"}]}]}}"#).unwrap();
        install_hooks_at(&home).unwrap();
        install_hooks_at(&home).unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
        assert_eq!(
            config["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "existing-hook"
        );
        assert_eq!(config["hooks"]["Stop"].as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn install_preserves_private_mode_and_symlink_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let home =
            std::env::temp_dir().join(format!("cowchat-codex-link-{}", uuid::Uuid::new_v4()));
        let codex = home.join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        let target = home.join("private-hooks.json");
        std::fs::write(&target, r#"{"hooks":{}}"#).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = codex.join("hooks.json");
        symlink(&target, &link).unwrap();
        install_hooks_at(&home).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&target).unwrap()).unwrap()
                ["hooks"]["Stop"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
