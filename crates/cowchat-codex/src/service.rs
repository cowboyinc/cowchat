use crate::app_server::{AppServerError, CodexWakeOutcome, WakeBackend, WakeReference};
use crate::config::{BridgeConfig, ConfigError, CowchatConfig, WakeHint};
use crate::store::{EventReservation, StoreError, WakeStore};
use async_trait::async_trait;
use cowchat_client::{ClientError, CowchatClient};
use cowchat_core::ChatMessage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_EVENT_BYTES: usize = 256 * 1024;
const MAX_READ_LIMIT: u32 = 500;
const WAKE_KIND: &str = "agent_wake";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WakeEvent {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub time: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeAgentInput {
    /// Configured recipient alias. Raw Codex thread ids are never accepted.
    pub target: String,
    pub source: String,
    pub event_id: String,
    pub event_type: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub wake_hint: WakeHint,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WakeAgentOutput {
    pub accepted: bool,
    pub duplicate: bool,
    pub target: String,
    pub room: String,
    pub seq: i64,
    pub wake: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexWakeOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeInboxReadInput {
    pub target: String,
    /// Defaults to the highest cursor previously acknowledged for this target.
    #[serde(default)]
    pub after_cursor: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WakeInboxItem {
    pub seq: i64,
    pub message_id: String,
    pub sender: String,
    pub event: WakeEvent,
    pub wake_hint: WakeHint,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WakeInboxReadOutput {
    pub target: String,
    pub room: String,
    pub after_cursor: i64,
    pub highest_returned_seq: i64,
    pub events: Vec<WakeInboxItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeInboxAckInput {
    pub target: String,
    /// Highest Cowchat room sequence that the agent has actually processed.
    pub cursor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WakeInboxAckOutput {
    pub target: String,
    pub last_acked_seq: i64,
    pub max_read_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_pending_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followup_wake: Option<String>,
}

#[async_trait]
pub trait ChatBackend: Send + Sync {
    async fn send_event(
        &self,
        target: &str,
        room: &str,
        event: &WakeEvent,
        hint: WakeHint,
    ) -> Result<ChatMessage, ServiceError>;

    async fn find_event(
        &self,
        target: &str,
        room: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<ChatMessage>, ServiceError>;

    async fn read_events(
        &self,
        target: &str,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, ServiceError>;
}

#[derive(Clone)]
pub struct CowchatBackend {
    config: CowchatConfig,
}

impl CowchatBackend {
    pub fn new(config: CowchatConfig) -> Self {
        Self { config }
    }

    async fn connect(&self) -> Result<CowchatClient, ServiceError> {
        let key =
            std::fs::read_to_string(&self.config.api_key_file).map_err(ServiceError::ReadApiKey)?;
        let key = key.trim();
        if key.is_empty() {
            return Err(ServiceError::EmptyApiKey);
        }
        let mut client = if let Some(socket) = &self.config.socket {
            CowchatClient::connect_uds(
                socket,
                key,
                &self.config.agent_name,
                None,
                vec!["codex-wake".into()],
            )
            .await?
        } else if let Some(tcp) = &self.config.tcp {
            CowchatClient::connect_tcp(
                tcp,
                key,
                &self.config.agent_name,
                None,
                vec!["codex-wake".into()],
            )
            .await?
        } else {
            return Err(ServiceError::InvalidCowchatTransport);
        };
        if let Some(env_name) = &self.config.room_key_env {
            let secret = std::env::var(env_name)
                .map_err(|_| ServiceError::MissingRoomKey(env_name.clone()))?;
            client.set_room_secret(secret.as_bytes());
        }
        Ok(client)
    }

    async fn joined_client(&self, room: &str) -> Result<CowchatClient, ServiceError> {
        let client = self.connect().await?;
        client.join_room(room).await?;
        Ok(client)
    }
}

#[async_trait]
impl ChatBackend for CowchatBackend {
    async fn send_event(
        &self,
        target: &str,
        room: &str,
        event: &WakeEvent,
        hint: WakeHint,
    ) -> Result<ChatMessage, ServiceError> {
        let client = self.joined_client(room).await?;
        let content = serde_json::to_string(event)?;
        let metadata = wake_metadata(target, event, hint);
        Ok(client
            .send_message_with_metadata(room, &content, None, Vec::new(), metadata)
            .await?)
    }

    async fn find_event(
        &self,
        target: &str,
        room: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<ChatMessage>, ServiceError> {
        let client = self.joined_client(room).await?;
        let mut cursor = 0;
        loop {
            let messages = client
                .get_history_filtered(room, 1000, None, None, Some(cursor))
                .await?;
            if messages.is_empty() {
                return Ok(None);
            }
            if let Some(message) = messages.iter().find(|message| {
                is_wake_for(message, target)
                    && metadata_string(message, "wake_source") == Some(source)
                    && metadata_string(message, "wake_event_id") == Some(event_id)
            }) {
                return Ok(Some(message.clone()));
            }
            cursor = messages.last().map(|message| message.seq).unwrap_or(cursor);
            if messages.len() < 1000 {
                return Ok(None);
            }
        }
    }

    async fn read_events(
        &self,
        target: &str,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, ServiceError> {
        let client = self.joined_client(room).await?;
        let mut cursor = after_seq;
        let mut result = Vec::new();
        while result.len() < limit as usize {
            let messages = client
                .get_history_filtered(room, 1000, None, None, Some(cursor))
                .await?;
            if messages.is_empty() {
                break;
            }
            cursor = messages.last().map(|message| message.seq).unwrap_or(cursor);
            for message in messages
                .iter()
                .filter(|message| is_wake_for(message, target))
            {
                result.push(message.clone());
                if result.len() == limit as usize {
                    break;
                }
            }
            if messages.len() < 1000 {
                break;
            }
        }
        Ok(result)
    }
}

pub struct WakeService {
    config: Arc<BridgeConfig>,
    store: Arc<WakeStore>,
    chat: Arc<dyn ChatBackend>,
    codex: Arc<dyn WakeBackend>,
}

impl Clone for WakeService {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            store: self.store.clone(),
            chat: self.chat.clone(),
            codex: self.codex.clone(),
        }
    }
}

impl WakeService {
    pub fn new(
        config: BridgeConfig,
        store: Arc<WakeStore>,
        chat: Arc<dyn ChatBackend>,
        codex: Arc<dyn WakeBackend>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            store,
            chat,
            codex,
        }
    }

    pub async fn wake_agent(&self, input: WakeAgentInput) -> Result<WakeAgentOutput, ServiceError> {
        validate_identifier("target", &input.target)?;
        validate_identifier("source", &input.source)?;
        validate_identifier("event_id", &input.event_id)?;
        validate_identifier("event_type", &input.event_type)?;
        let target = self.config.target(&input.target)?.clone();
        let event_time = input
            .time
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        chrono::DateTime::parse_from_rfc3339(&event_time)
            .map_err(|_| ServiceError::InvalidEventTime(event_time.clone()))?;
        let event = WakeEvent {
            specversion: "1.0".into(),
            id: input.event_id.clone(),
            source: input.source.clone(),
            event_type: input.event_type.clone(),
            subject: input.subject,
            time: event_time,
            data: input.data,
        };
        let event_json = serde_json::to_string(&event)?;
        if event_json.len() > MAX_EVENT_BYTES {
            return Err(ServiceError::EventTooLarge(event_json.len()));
        }

        let reservation = self.store.reserve_event(EventReservation {
            target: &input.target,
            source: &event.source,
            event_id: &event.id,
            event_json: &event_json,
            room_id: &target.room,
            wake_hint_rank: wake_hint_rank(input.wake_hint),
            now_unix: chrono::Utc::now().timestamp(),
        })?;
        let message = if let Some(seq) = reservation.room_seq {
            Some(ChatMessage {
                message_id: String::new(),
                room_id: target.room.clone(),
                agent_id: String::new(),
                agent_name: String::new(),
                content: event_json.clone(),
                reply_to_message: None,
                metadata: wake_metadata(&input.target, &event, input.wake_hint),
                timestamp: chrono::Utc::now(),
                seq,
            })
        } else if let Some(recovered) = self
            .chat
            .find_event(&input.target, &target.room, &event.source, &event.id)
            .await?
        {
            self.store.mark_delivered(
                &input.target,
                &event.source,
                &event.id,
                recovered.seq,
                &recovered.message_id,
            )?;
            Some(recovered)
        } else {
            let sent = self
                .chat
                .send_event(&input.target, &target.room, &event, input.wake_hint)
                .await?;
            self.store.mark_delivered(
                &input.target,
                &event.source,
                &event.id,
                sent.seq,
                &sent.message_id,
            )?;
            Some(sent)
        };
        let seq = message
            .as_ref()
            .map(|message| message.seq)
            .unwrap_or_default();

        if input.wake_hint < target.min_wake_hint {
            return Ok(WakeAgentOutput {
                accepted: true,
                duplicate: reservation.duplicate,
                target: input.target,
                room: target.room,
                seq,
                wake: "filtered_by_recipient_policy".into(),
                codex: None,
            });
        }
        let codex = self
            .maybe_wake(&input.target, &target.thread_id, &target.room, seq, &event)
            .await?;
        Ok(WakeAgentOutput {
            accepted: true,
            duplicate: reservation.duplicate,
            target: input.target,
            room: target.room,
            seq,
            wake: if codex.is_some() {
                "triggered".into()
            } else {
                "coalesced".into()
            },
            codex,
        })
    }

    pub async fn read_inbox(
        &self,
        input: WakeInboxReadInput,
    ) -> Result<WakeInboxReadOutput, ServiceError> {
        let target = self.config.target(&input.target)?.clone();
        let after_cursor = input
            .after_cursor
            .unwrap_or(self.store.last_acked_seq(&input.target)?);
        if after_cursor < 0 {
            return Err(ServiceError::InvalidCursor(after_cursor));
        }
        let limit = input.limit.unwrap_or(100).clamp(1, MAX_READ_LIMIT);
        let messages = self
            .chat
            .read_events(&input.target, &target.room, after_cursor, limit)
            .await?;
        let mut events = Vec::with_capacity(messages.len());
        for message in messages {
            let wake_hint = metadata_wake_hint(&message);
            let event: WakeEvent = serde_json::from_str(&message.content).map_err(|source| {
                ServiceError::InvalidStoredEvent {
                    seq: message.seq,
                    source,
                }
            })?;
            events.push(WakeInboxItem {
                seq: message.seq,
                message_id: message.message_id,
                sender: message.agent_name,
                event,
                wake_hint,
            });
        }
        let highest_returned_seq = events.last().map(|event| event.seq).unwrap_or(after_cursor);
        self.store
            .record_read(&input.target, highest_returned_seq)?;
        Ok(WakeInboxReadOutput {
            target: input.target,
            room: target.room,
            after_cursor,
            highest_returned_seq,
            events,
        })
    }

    pub async fn acknowledge(
        &self,
        input: WakeInboxAckInput,
    ) -> Result<WakeInboxAckOutput, ServiceError> {
        let target = self.config.target(&input.target)?.clone();
        let state = self.store.acknowledge(&input.target, input.cursor)?;
        let mut followup_wake = None;
        if let Some(seq) = self
            .store
            .max_pending_eligible_seq(&input.target, wake_hint_rank(target.min_wake_hint))?
        {
            if let Some(record) = self.store.delivered_event(&input.target, seq)? {
                let event: WakeEvent = serde_json::from_str(&record.event_json)?;
                let outcome = self
                    .maybe_wake(&input.target, &target.thread_id, &target.room, seq, &event)
                    .await?;
                followup_wake = Some(if outcome.is_some() {
                    "triggered".into()
                } else {
                    "coalesced".into()
                });
            }
        }
        Ok(WakeInboxAckOutput {
            target: input.target,
            last_acked_seq: state.last_acked_seq,
            max_read_seq: state.max_read_seq,
            next_pending_seq: state.max_pending_seq,
            followup_wake,
        })
    }

    async fn maybe_wake(
        &self,
        alias: &str,
        thread_id: &str,
        room: &str,
        observed_seq: i64,
        event: &WakeEvent,
    ) -> Result<Option<CodexWakeOutcome>, ServiceError> {
        let claimed = self.store.claim_wake(
            alias,
            observed_seq,
            chrono::Utc::now().timestamp(),
            self.config.codex.wake_lease_seconds,
        )?;
        if !claimed {
            return Ok(None);
        }
        let reference = WakeReference {
            target: alias.to_string(),
            room: room.to_string(),
            after_seq: self.store.last_acked_seq(alias)?,
            observed_seq,
            source: event.source.clone(),
            event_id: event.id.clone(),
            event_type: event.event_type.clone(),
        };
        match self.codex.wake(thread_id, &reference).await {
            Ok(outcome) => Ok(Some(outcome)),
            Err(error) => {
                self.store.release_wake(alias)?;
                Err(ServiceError::AppServer(error))
            }
        }
    }
}

fn wake_metadata(target: &str, event: &WakeEvent, hint: WakeHint) -> Value {
    json!({
        "kind": WAKE_KIND,
        "wake_target": target,
        "wake_source": event.source,
        "wake_event_id": event.id,
        "wake_event_type": event.event_type,
        "wake_hint": hint,
    })
}

fn is_wake_for(message: &ChatMessage, target: &str) -> bool {
    metadata_string(message, "kind") == Some(WAKE_KIND)
        && metadata_string(message, "wake_target") == Some(target)
}

fn metadata_string<'a>(message: &'a ChatMessage, key: &str) -> Option<&'a str> {
    message.metadata.get(key).and_then(Value::as_str)
}

fn metadata_wake_hint(message: &ChatMessage) -> WakeHint {
    message
        .metadata
        .get("wake_hint")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn wake_hint_rank(hint: WakeHint) -> i64 {
    match hint {
        WakeHint::None => 0,
        WakeHint::Normal => 1,
        WakeHint::Urgent => 2,
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ServiceError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ServiceError::InvalidIdentifier(field));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    AppServer(#[from] AppServerError),
    #[error("Cowchat client error: {0}")]
    Cowchat(#[from] ClientError),
    #[error("failed to read Cowchat API key: {0}")]
    ReadApiKey(#[source] std::io::Error),
    #[error("Cowchat API key file is empty")]
    EmptyApiKey,
    #[error("configure exactly one Cowchat transport")]
    InvalidCowchatTransport,
    #[error("environment variable {0} is required for the encrypted Cowchat room key")]
    MissingRoomKey(String),
    #[error("wake event is {0} bytes; maximum is 262144")]
    EventTooLarge(usize),
    #[error("{0} must be non-empty, at most 512 characters, and contain no control characters")]
    InvalidIdentifier(&'static str),
    #[error("cursor must be non-negative, got {0}")]
    InvalidCursor(i64),
    #[error("event time must be RFC3339, got {0:?}")]
    InvalidEventTime(String),
    #[error("Cowchat wake message at seq {seq} does not contain a valid event: {source}")]
    InvalidStoredEvent {
        seq: i64,
        #[source]
        source: serde_json::Error,
    },
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::CodexWakeOutcome;
    use crate::config::{CodexConfig, TargetConfig};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeChat {
        messages: Mutex<Vec<ChatMessage>>,
    }

    #[async_trait]
    impl ChatBackend for FakeChat {
        async fn send_event(
            &self,
            target: &str,
            room: &str,
            event: &WakeEvent,
            hint: WakeHint,
        ) -> Result<ChatMessage, ServiceError> {
            let mut messages = self.messages.lock().unwrap();
            let message = ChatMessage {
                message_id: format!("msg-{}", messages.len() + 1),
                room_id: room.into(),
                agent_id: "sender-id".into(),
                agent_name: "sender".into(),
                content: serde_json::to_string(event).unwrap(),
                reply_to_message: None,
                metadata: wake_metadata(target, event, hint),
                timestamp: chrono::Utc::now(),
                seq: messages.len() as i64 + 1,
            };
            messages.push(message.clone());
            Ok(message)
        }

        async fn find_event(
            &self,
            target: &str,
            _room: &str,
            source: &str,
            event_id: &str,
        ) -> Result<Option<ChatMessage>, ServiceError> {
            Ok(self
                .messages
                .lock()
                .unwrap()
                .iter()
                .find(|message| {
                    is_wake_for(message, target)
                        && metadata_string(message, "wake_source") == Some(source)
                        && metadata_string(message, "wake_event_id") == Some(event_id)
                })
                .cloned())
        }

        async fn read_events(
            &self,
            target: &str,
            _room: &str,
            after_seq: i64,
            limit: u32,
        ) -> Result<Vec<ChatMessage>, ServiceError> {
            Ok(self
                .messages
                .lock()
                .unwrap()
                .iter()
                .filter(|message| message.seq > after_seq && is_wake_for(message, target))
                .take(limit as usize)
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct FakeWake {
        calls: Mutex<Vec<WakeReference>>,
    }

    #[async_trait]
    impl WakeBackend for FakeWake {
        async fn wake(
            &self,
            _thread_id: &str,
            reference: &WakeReference,
        ) -> Result<CodexWakeOutcome, AppServerError> {
            self.calls.lock().unwrap().push(reference.clone());
            Ok(CodexWakeOutcome {
                mode: "started".into(),
                prior_status: "idle".into(),
                turn_id: "turn-1".into(),
            })
        }
    }

    fn test_config(min_wake_hint: WakeHint) -> BridgeConfig {
        BridgeConfig {
            state_db: "unused".into(),
            cowchat: CowchatConfig::default(),
            codex: CodexConfig {
                app_server_endpoint: "ws://unused".into(),
                bearer_token_env: None,
                request_timeout_seconds: 1,
                wake_lease_seconds: 30,
            },
            targets: BTreeMap::from([(
                "reviewer".into(),
                TargetConfig {
                    thread_id: "thr-1".into(),
                    room: "room".into(),
                    min_wake_hint,
                },
            )]),
        }
    }

    fn wake_input(id: &str, hint: WakeHint) -> WakeAgentInput {
        WakeAgentInput {
            target: "reviewer".into(),
            source: "ci".into(),
            event_id: id.into(),
            event_type: "build.completed".into(),
            subject: None,
            time: Some("2026-08-02T00:00:00Z".into()),
            data: json!({"status": "green"}),
            wake_hint: hint,
        }
    }

    #[tokio::test]
    async fn duplicate_is_stored_once_and_wake_is_coalesced() {
        let chat = Arc::new(FakeChat::default());
        let wake = Arc::new(FakeWake::default());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            wake.clone(),
        );
        let first = service
            .wake_agent(wake_input("evt-1", WakeHint::Normal))
            .await
            .unwrap();
        let duplicate = service
            .wake_agent(wake_input("evt-1", WakeHint::Normal))
            .await
            .unwrap();
        assert_eq!(first.wake, "triggered");
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.wake, "coalesced");
        assert_eq!(chat.messages.lock().unwrap().len(), 1);
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_non_rfc3339_event_time_before_delivery() {
        let chat = Arc::new(FakeChat::default());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        let mut input = wake_input("evt-1", WakeHint::Normal);
        input.time = Some("tomorrow-ish".into());
        assert!(matches!(
            service.wake_agent(input).await,
            Err(ServiceError::InvalidEventTime(_))
        ));
        assert!(chat.messages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn recipient_policy_filters_wake_but_keeps_durable_event() {
        let chat = Arc::new(FakeChat::default());
        let wake = Arc::new(FakeWake::default());
        let service = WakeService::new(
            test_config(WakeHint::Urgent),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat,
            wake.clone(),
        );
        let result = service
            .wake_agent(wake_input("evt-1", WakeHint::Normal))
            .await
            .unwrap();
        assert_eq!(result.wake, "filtered_by_recipient_policy");
        assert!(wake.calls.lock().unwrap().is_empty());
        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(inbox.events.len(), 1);
    }

    #[tokio::test]
    async fn acknowledgement_does_not_wake_for_filtered_pending_event() {
        let wake = Arc::new(FakeWake::default());
        let service = WakeService::new(
            test_config(WakeHint::Urgent),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            Arc::new(FakeChat::default()),
            wake.clone(),
        );
        service
            .wake_agent(wake_input("evt-1", WakeHint::Urgent))
            .await
            .unwrap();
        service
            .wake_agent(wake_input("evt-2", WakeHint::Normal))
            .await
            .unwrap();
        service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        let ack = service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                cursor: 1,
            })
            .await
            .unwrap();
        assert_eq!(ack.next_pending_seq, Some(2));
        assert_eq!(ack.followup_wake, None);
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ack_requires_a_processed_cursor() {
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            Arc::new(FakeChat::default()),
            Arc::new(FakeWake::default()),
        );
        service
            .wake_agent(wake_input("evt-1", WakeHint::Normal))
            .await
            .unwrap();
        assert!(service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                cursor: 1,
            })
            .await
            .is_err());
        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        let ack = service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                cursor: inbox.highest_returned_seq,
            })
            .await
            .unwrap();
        assert_eq!(ack.last_acked_seq, 1);
        assert_eq!(ack.next_pending_seq, None);
    }
}
