//! Connector-neutral room delivery. Providers own ingress authentication,
//! external acknowledgements, target allowlists and outbound chunking.
use super::*;
use cowchat_core::SendMessagePayload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Accepted provider event. Identity labels are placed inside the ciphertext.
/// Ingress must authenticate, allowlist and durably deduplicate completed events
/// before handing them here. A deterministic ID alone does not make a newly
/// encrypted redelivery match the already committed ciphertext.
pub(super) struct ExternalMessage {
    pub message_id: String,
    pub sender: String,
    pub text: String,
}

/// Return success only after the configured destination accepts all text.
/// Ingress calls RoomBridge::receive serially, regardless of its transport.
pub(super) trait Connector {
    async fn send(&self, text: &str) -> Result<()>;
}

pub(super) struct RoomBridge {
    pub room: String,
    pub secret: Vec<u8>,
    pub state_file: PathBuf,
    pub mentions: Vec<String>,
    pub skip_room_seq: Option<i64>,
}

impl RoomBridge {
    pub fn prepare(&self, event: ExternalMessage, next_cursor: Option<Value>) -> Pending {
        Pending {
            next_cursor,
            message: SendMessagePayload {
                message_id: Some(event.message_id),
                room_id: self.room.clone(),
                content: cowchat_core::crypto::encrypt(
                    &self.secret,
                    &self.room,
                    &format!("{}: {}", event.sender, event.text),
                ),
                reply_to: None,
                metadata: json!({"bridge": true}),
                mentions: self.mentions.clone(),
            },
        }
    }

    /// Caller may acknowledge an external event only after this returns success.
    pub async fn receive(
        &self,
        client: &CowchatClient,
        state: &mut State,
        event: ExternalMessage,
        next_cursor: Option<Value>,
    ) -> Result<()> {
        flush_pending(client, state, &self.state_file).await?;
        state.pending = Some(self.prepare(event, next_cursor));
        save(&self.state_file, state)?;
        flush_pending(client, state, &self.state_file).await
    }

    pub async fn forward(
        &self,
        client: &CowchatClient,
        connector: &impl Connector,
        state: &mut State,
    ) -> Result<()> {
        // Raw client has no room secret; failures must never expose ciphertext.
        let history = client
            .get_history_filtered(&self.room, 100, None, None, Some(state.room_seq))
            .await
            .map_err(|_| "Cowchat replay failed")?;
        for message in history {
            if message.agent_id != client.agent_id
                && !matches!(
                    message.metadata["type"].as_str(),
                    Some("thinking" | "system")
                )
            {
                let text = match cowchat_core::crypto::decrypt(
                    &self.secret,
                    &self.room,
                    &message.content,
                ) {
                    Ok(plain) => format!(
                        "{}: {}",
                        message.agent_name.chars().take(128).collect::<String>(),
                        plain
                    ),
                    Err(_) if self.skip_room_seq == Some(message.seq) => format!(
                        "[Bridge operator skipped unreadable room message at sequence {}]",
                        message.seq
                    ),
                    Err(_) => {
                        eprintln!("Bridge blocked at room sequence {}: unable to decrypt. Restore the correct room key or explicitly skip this sequence.", message.seq);
                        if state.reported_block != Some(message.seq) {
                            connector.send(&format!("[Bridge paused at room sequence {}: message could not be decrypted. Operator action required.]", message.seq)).await?;
                            state.reported_block = Some(message.seq);
                            save(&self.state_file, state)?;
                        }
                        return Err(
                            "Room message could not be decrypted; cursor not advanced".into()
                        );
                    }
                };
                connector.send(&text).await?;
            }
            state.room_seq = message.seq;
            state.reported_block = None;
            save(&self.state_file, state)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct State {
    pub(super) binding: String,
    pub(super) external_cursor: Option<Value>,
    pub(super) room_seq: i64,
    pub(super) pending: Option<Pending>,
    #[serde(default)]
    pub(super) reported_block: Option<i64>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Pending {
    pub(super) next_cursor: Option<Value>,
    pub(super) message: SendMessagePayload,
}

pub(super) fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

pub(super) fn save(path: &Path, state: &State) -> Result<()> {
    let temporary = sidecar(path, ".tmp");
    let mut file = private_file(&temporary)?;
    file.set_len(0)?;
    file.write_all(&serde_json::to_vec(state)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    #[cfg(unix)]
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?
    .sync_all()?;
    Ok(())
}

pub(super) async fn flush_pending(
    client: &CowchatClient,
    state: &mut State,
    path: &Path,
) -> Result<()> {
    if let Some(pending) = &state.pending {
        client
            .append_prepared_message(&pending.message)
            .await
            .map_err(|_| "Cowchat append failed; pending ciphertext retained")?;
        state.external_cursor = pending.next_cursor.clone();
        state.pending = None;
        save(path, state)?;
    }
    Ok(())
}
