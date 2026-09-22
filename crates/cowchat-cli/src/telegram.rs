//! One explicitly configured Telegram chat <-> encrypted Cowchat room.
//! The bridge and Telegram see plaintext; Cowchat storage does not.
use super::bridge::{self, Connector, ExternalMessage, Result, RoomBridge, State};
use super::*;
use bridge::{flush_pending, private_file, save, sidecar};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(clap::Args)]
pub(crate) struct TelegramArgs {
    pub room: String,
    /// The only Telegram chat this bridge may read from or send to.
    #[arg(long, allow_hyphen_values = true)]
    pub chat_id: i64,
    /// Dedicated durable cursor file. Use one bridge process per bot.
    #[arg(long)]
    pub state_file: PathBuf,
    /// Actor IDs addressed by incoming Telegram text (repeatable).
    #[arg(long)]
    pub mention: Vec<String>,
    /// Explicitly skip only this unreadable outbound sequence, reporting the gap.
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
    pub skip_room_seq: Option<i64>,
}

struct Telegram {
    http: reqwest::Client,
    base: String,
    chat_id: i64,
}
impl Telegram {
    async fn call(&self, method: &str, payload: Value) -> Result<Value> {
        // reqwest errors contain the request URL (which contains the bot token).
        // Never propagate those errors or Telegram's arbitrary description text.
        let response = self
            .http
            .post(format!("{}/{method}", self.base))
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&payload)?)
            .send()
            .await
            .map_err(|_| "Telegram transport failed")?;
        let successful_status = response.status().is_success();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "Telegram response failed")?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid Telegram response")?;
        if !successful_status || value["ok"] != true {
            if value["error_code"] == 429 {
                let delay = value["parameters"]["retry_after"]
                    .as_u64()
                    .unwrap_or(5)
                    .clamp(1, 300);
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
            return Err("Telegram rejected request; check bot access, webhook/polling configuration and chat permissions".into());
        }
        Ok(value["result"].clone())
    }
}

fn inbound(
    update: &Value,
    chat_id: i64,
    agent: &str,
    bot_id: &str,
) -> Result<Option<ExternalMessage>> {
    let update_id = update["update_id"]
        .as_i64()
        .filter(|id| *id >= 0)
        .ok_or("Invalid Telegram update ID")?;
    update_id
        .checked_add(1)
        .ok_or("Telegram update ID overflow")?;
    let message = &update["message"];
    if message["chat"]["id"].as_i64() != Some(chat_id)
        || message["from"]["is_bot"] != false
        || message["text"].as_str().is_none()
    {
        return Ok(None);
    }
    let sender = message["from"]["id"]
        .as_i64()
        .ok_or("Invalid Telegram sender")?;
    Ok(Some(ExternalMessage {
        message_id: format!("telegram:{agent}:{bot_id}:{update_id}"),
        sender: format!("Telegram user {sender}"),
        text: message["text"].as_str().unwrap().into(),
    }))
}

impl Connector for Telegram {
    async fn send(&self, text: &str) -> Result<()> {
        for part in chunks(text) {
            self.call(
                "sendMessage",
                json!({"chat_id": self.chat_id, "text": part,
                "link_preview_options": {"is_disabled": true}}),
            )
            .await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }
}

fn chunks(text: &str) -> Vec<String> {
    // Telegram counts text length after parsing. No parse_mode is enabled;
    // use a conservative UTF-16 bound so emoji cannot exceed the limit.
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut units = 0;
    for c in text.chars() {
        if units + c.len_utf16() > 4000 {
            parts.push(std::mem::take(&mut part));
            units = 0;
        }
        part.push(c);
        units += c.len_utf16();
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

async fn pump(
    client: &CowchatClient,
    telegram: &Telegram,
    bridge: &RoomBridge,
    state: &mut State,
    bot_id: &str,
) -> Result<()> {
    flush_pending(client, state, &bridge.state_file).await?;
    let offset = state
        .external_cursor
        .as_ref()
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let updates = telegram
        .call(
            "getUpdates",
            json!({"offset": offset, "timeout": 2,
        "limit": 100, "allowed_updates": ["message"]}),
        )
        .await?;
    for update in updates.as_array().ok_or("Invalid Telegram updates")? {
        let id = update["update_id"]
            .as_i64()
            .filter(|id| *id >= 0)
            .ok_or("Invalid Telegram update ID")?;
        if id
            < state
                .external_cursor
                .as_ref()
                .and_then(Value::as_i64)
                .unwrap_or(0)
        {
            continue;
        }
        let next = Some(json!(id
            .checked_add(1)
            .ok_or("Telegram update ID overflow")?));
        if let Some(event) = inbound(update, telegram.chat_id, &client.agent_id, bot_id)? {
            bridge.receive(client, state, event, next).await?;
        } else {
            state.external_cursor = next;
            save(&bridge.state_file, state)?;
        }
    }
    bridge.forward(client, telegram, state).await
}

async fn raw_client(cli: &Cli) -> Result<CowchatClient> {
    let key = load_key(&cli.key);
    let agent = resolve_agent_id(cli).ok_or("telegram-bridge requires --agent-id")?;
    Ok(if let Some(url) = &cli.url {
        CowchatClient::connect_ws(url, &key, &cli.name, Some(&agent), vec![]).await?
    } else if let Some(addr) = &cli.tcp {
        CowchatClient::connect_tcp(addr, &key, &cli.name, Some(&agent), vec![]).await?
    } else {
        CowchatClient::connect_uds(&cli.socket, &key, &cli.name, Some(&agent), vec![]).await?
    })
}

pub(crate) async fn run(cli: &Cli, args: &TelegramArgs) -> Result<()> {
    let token = env_non_empty("TELEGRAM_BOT_TOKEN")
        .ok_or("Set TELEGRAM_BOT_TOKEN for a dedicated bridge bot")?;
    let bot_id = token
        .split_once(':')
        .map(|p| p.0)
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .ok_or("Invalid Telegram bot token format")?;
    let secret = resolve_room_secret(cli).ok_or("telegram-bridge requires COWCHAT_ROOM_KEY")?;
    let _lock = private_file(&sidecar(&args.state_file, ".lock"))?;
    _lock
        .try_lock()
        .map_err(|_| "Another bridge is using this state file")?;
    let mut client = raw_client(cli).await?;
    let room = resolve_room_id(&client, &args.room).await?;
    if client.room_info(&room).await?["room"]["encrypted"] != true {
        return Err("telegram-bridge requires an encrypted Cowchat room".into());
    }
    client.join_room(&room).await?;
    let recent = client.get_history(&room, 20, None).await?;
    let ciphertexts: Vec<_> = recent
        .iter()
        .filter(|m| cowchat_core::crypto::is_ciphertext(&m.content))
        .collect();
    if !ciphertexts.is_empty()
        && !ciphertexts
            .iter()
            .any(|m| cowchat_core::crypto::decrypt(&secret, &room, &m.content).is_ok())
        && args.skip_room_seq.is_none()
    {
        return Err("Room key cannot decrypt recent history; restore the key or explicitly skip one unreadable sequence".into());
    }
    let binding = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([
            "telegram",
            cli.url,
            cli.tcp,
            cli.socket,
            client.agent_id,
            room,
            args.chat_id,
            bot_id,
            args.mention,
            load_key(&cli.key)
        ]))?)
    );
    let mut state: State = match std::fs::read(&args.state_file) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => State {
            binding: binding.clone(),
            external_cursor: None,
            room_seq: client.room_tip(&room).await?,
            pending: None,
            reported_block: None,
        },
        Err(error) => return Err(error.into()),
    };
    if state.binding != binding {
        return Err("Bridge state belongs to a different mapping or identity".into());
    }
    save(&args.state_file, &state)?;
    let telegram = Telegram {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        base: format!("https://api.telegram.org/bot{token}"),
        chat_id: args.chat_id,
    };
    let bridge = RoomBridge {
        room: room.clone(),
        secret,
        state_file: args.state_file.clone(),
        mentions: args.mention.clone(),
        skip_room_seq: args.skip_room_seq,
    };
    eprintln!("Telegram bridge ready; Telegram and this bridge can read relayed plaintext. No earlier room history is exported on first start.");
    loop {
        let result = tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            result = pump(&client, &telegram, &bridge, &mut state, bot_id) => result,
        };
        if result.is_err() {
            // Never log remote error bodies, tokens, room keys, or message text.
            eprintln!("Bridge delivery paused; retrying from saved state. Check connectivity, room key and Telegram bot configuration.");
            tokio::time::sleep(Duration::from_secs(3)).await;
            // Re-read before retry: an interrupted state save must not acknowledge
            // data that only existed in this process's memory.
            state = serde_json::from_slice(&std::fs::read(&args.state_file)?)?;
            match raw_client(cli).await {
                Ok(next) => {
                    next.join_room(&room).await?;
                    client = next;
                }
                Err(_) => continue,
            }
        }
    }
}

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;
