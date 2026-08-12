use clap::{Parser, Subcommand};
use cowchat_codex::app_server::CodexAppServerClient;
use cowchat_codex::config::{BridgeConfig, BridgeRole};
use cowchat_codex::mcp::{serve_stdio, WakeMcpServer};
use cowchat_codex::relay::{RelayChatBackend, WakeRelay};
use cowchat_codex::service::{ChatBackend, CowchatBackend, WakeService};
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
    Doctor {
        /// Connect to Cowchat, resolve every configured canonical room, and
        /// read each Codex thread without waking it.
        #[arg(long)]
        live: bool,
    },
    /// Watch explicitly enabled target rooms and wake ended Codex tasks for
    /// ordinary peer messages.
    Relay {
        /// Process the durable backlog once, then exit.
        #[arg(long)]
        once: bool,
        /// On a target's first run, process existing history instead of
        /// seeding its cursor at the current room tip.
        #[arg(long)]
        from_start: bool,
    },
    /// Safely clear persisted state for one configured target in the current
    /// endpoint/config scope. Cowchat messages are never deleted.
    ResetState {
        /// Configured target alias whose local cursor/idempotency state resets.
        #[arg(long)]
        target: String,
        /// Explicitly discard pre-v0.7 state for this alias when it cannot be
        /// migrated. Durable Cowchat history is retained but the old wake
        /// generation will no longer be readable through the bridge.
        #[arg(long)]
        discard_legacy_state: bool,
    },
    /// Transactionally bind and migrate one pre-v0.7 target using the current
    /// configured identity and canonical room.
    MigrateLegacyState {
        /// Configured target alias whose delivered/unacknowledged events migrate.
        #[arg(long)]
        target: String,
    },
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
    let scope = config.state_scope();
    let legacy_maintenance = matches!(
        &cli.command,
        Command::MigrateLegacyState { .. }
            | Command::ResetState {
                discard_legacy_state: true,
                ..
            }
    );
    let store = Arc::new(if legacy_maintenance {
        WakeStore::open_for_legacy_maintenance(&config.state_db, &scope)?
    } else {
        WakeStore::open(&config.state_db, &scope)?
    });
    if let Command::MigrateLegacyState { target } = &cli.command {
        let target_config = config.target(target)?;
        let identity = config.target_identity(target)?;
        let chat = CowchatBackend::for_role(config.cowchat.clone(), BridgeRole::Doctor);
        let room = chat.inspect_room(&target_config.room).await?;
        if room.ephemeral {
            return Err(format!(
                "Cowchat room {:?} is temporary; legacy wake state can only migrate against a permanent durable room",
                target_config.room
            )
            .into());
        }
        let _target_lock = store.lock_target_exclusive_async(target).await?;
        let handle =
            store.migrate_legacy_target(&identity, target, &target_config.room, room.tip)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "target": target,
                "state_id": handle.state_id,
                "floor_seq": handle.floor_seq,
                "scope": scope,
                "state_db": config.state_db,
                "note": "migrated this target transactionally; delivered but unacknowledged v0.6 events remain pending and will be returned again"
            }))?
        );
        return Ok(());
    }
    if let Command::ResetState {
        target,
        discard_legacy_state,
    } = &cli.command
    {
        let target_config = config.target(target)?;
        let identity = config.target_identity(target)?;
        let chat = CowchatBackend::for_role(config.cowchat.clone(), BridgeRole::Doctor);
        let room = chat.inspect_room(&target_config.room).await?;
        if room.ephemeral {
            return Err(format!(
                "Cowchat room {:?} is temporary; wake state can only reset against a permanent durable room",
                target_config.room
            )
            .into());
        }
        let _target_lock = store.lock_target_exclusive_async(target).await?;
        let handle = if *discard_legacy_state {
            store.reset_target_discarding_legacy(
                &identity,
                target,
                &target_config.room,
                room.tip,
            )?
        } else {
            store.reset_target(&identity, target, &target_config.room, room.tip)?
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "target": target,
                "state_id": handle.state_id,
                "floor_seq": handle.floor_seq,
                "scope": scope,
                "state_db": config.state_db,
                "legacy_state_discarded": discard_legacy_state,
                "note": if *discard_legacy_state {
                    "explicitly discarded this target's pre-v0.7 local state and rotated at the verified live room tip; durable Cowchat room history was not modified"
                } else {
                    "rotated only this target at the verified live room tip; durable Cowchat room history was not modified"
                }
            }))?
        );
        return Ok(());
    }
    if let Command::Doctor { live } = &cli.command {
        let mut live_targets = Vec::new();
        let mut all_ready = true;
        if *live {
            let chat = CowchatBackend::for_role(config.cowchat.clone(), BridgeRole::Doctor);
            let codex = CodexAppServerClient::new(config.codex.clone());
            for (alias, target) in &config.targets {
                let room = chat.inspect_room(&target.room).await;
                let room_tip = chat.room_tip(&target.room).await;
                let thread = codex.inspect_thread(&target.thread_id).await;
                let ready = room.as_ref().is_ok_and(|room| !room.ephemeral)
                    && room_tip.is_ok()
                    && thread.as_ref().is_ok_and(|thread| thread.ready);
                all_ready &= ready;
                live_targets.push(json!({
                    "target": alias,
                    "room": target.room,
                    "room_readiness": room.as_ref().ok(),
                    "room_error": room.as_ref().err().map(ToString::to_string),
                    "room_tip": room_tip.as_ref().ok(),
                    "room_tip_error": room_tip.as_ref().err().map(ToString::to_string),
                    "thread_id": target.thread_id,
                    "thread_readiness": thread.as_ref().ok(),
                    "thread_error": thread.as_ref().err().map(ToString::to_string),
                    "ready": ready,
                }));
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": all_ready,
                "config": cli.config,
                "state_db": config.state_db,
                "state_scope": scope,
                "bridge_role_ids": {
                    "mcp": config.cowchat.role_agent_id(BridgeRole::Mcp),
                    "relay": config.cowchat.role_agent_id(BridgeRole::Relay),
                    "doctor": config.cowchat.role_agent_id(BridgeRole::Doctor),
                },
                "targets": config.targets.keys().collect::<Vec<_>>(),
                "live": live,
                "live_targets": live_targets,
                "note": if *live {
                    "live doctor resolved every Cowchat room and read every Codex thread without waking a task"
                } else {
                    "local doctor validates configuration and the state database; add --live to verify Cowchat rooms and Codex threads"
                }
            }))?
        );
        if !all_ready {
            return Err(
                "one or more live targets are not ready; see structured doctor output".into(),
            );
        }
        return Ok(());
    }

    let role = if matches!(cli.command, Command::Relay { .. }) {
        BridgeRole::Relay
    } else {
        BridgeRole::Mcp
    };
    let chat = Arc::new(CowchatBackend::for_role(config.cowchat.clone(), role));
    let codex = Arc::new(CodexAppServerClient::new(config.codex.clone()));
    let service = WakeService::new(config.clone(), store.clone(), chat.clone(), codex);
    match cli.command {
        Command::Mcp => serve_stdio(WakeMcpServer::new(service)).await?,
        Command::Relay { once, from_start } => {
            let relay = WakeRelay::new(config, store, chat, service);
            if once {
                let relayed = relay.run_once(from_start).await?;
                println!("relayed {relayed} message(s)");
            } else {
                relay.run_forever(from_start).await?;
            }
        }
        Command::Doctor { .. }
        | Command::ConfigExample
        | Command::ResetState { .. }
        | Command::MigrateLegacyState { .. } => {
            unreachable!("handled before setup")
        }
    }
    Ok(())
}

fn default_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat/codex-wake.json"))
        .unwrap_or_else(|| PathBuf::from(".cowchat/codex-wake.json"))
}
