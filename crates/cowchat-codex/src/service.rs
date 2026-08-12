use crate::app_server::{AppServerError, CodexWakeOutcome, WakeBackend, WakeReference};
use crate::config::{BridgeConfig, BridgeRole, ConfigError, CowchatConfig, WakeHint};
use crate::store::{DeliveryClaim, EventReservation, StoreError, TargetHandle, WakeStore};
use async_trait::async_trait;
use cowchat_client::{ClientError, CowchatClient};
use cowchat_core::ChatMessage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

const MAX_EVENT_BYTES: usize = 256 * 1024;
const MAX_READ_LIMIT: u32 = 500;
const WAKE_KIND: &str = "agent_wake";
const CIPHERTEXT_PREFIX: &str = "cow1:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomReadiness {
    pub ephemeral: bool,
    pub encrypted: bool,
    pub key_validation: String,
    /// Durable room sequence at the time readiness was inspected.
    pub tip: i64,
}

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

#[derive(Debug, Clone, Copy)]
pub struct WakeEventLookup<'a> {
    pub event: &'a WakeEvent,
    pub hint: WakeHint,
    pub event_digest: &'a str,
    pub allow_legacy_metadata: bool,
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
    pub state_id: String,
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
    /// Required with an explicit cursor. Omit both fields to resume from the
    /// current target generation's acknowledged cursor.
    #[serde(default)]
    pub state_id: Option<String>,
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
    pub state_id: String,
    pub room: String,
    pub after_cursor: i64,
    pub highest_returned_seq: i64,
    pub events: Vec<WakeInboxItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeInboxAckInput {
    pub target: String,
    /// Target generation returned by `wake_inbox_read`.
    pub state_id: String,
    /// Highest Cowchat room sequence that the agent has actually processed.
    pub cursor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WakeInboxAckOutput {
    pub target: String,
    pub state_id: String,
    pub last_acked_seq: i64,
    pub max_read_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_pending_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followup_wake: Option<String>,
}

#[async_trait]
pub trait ChatBackend: Send + Sync {
    async fn inspect_room(&self, room: &str) -> Result<RoomReadiness, ServiceError>;

    async fn send_event(
        &self,
        target: &str,
        state_id: &str,
        room: &str,
        event: &WakeEvent,
        hint: WakeHint,
        event_digest: &str,
    ) -> Result<ChatMessage, ServiceError>;

    async fn find_event(
        &self,
        target: &str,
        state_id: &str,
        room: &str,
        lookup: WakeEventLookup<'_>,
    ) -> Result<Option<ChatMessage>, ServiceError>;

    async fn read_events(
        &self,
        target: &str,
        state_id: &str,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, ServiceError>;
}

#[derive(Clone)]
pub struct CowchatBackend {
    config: CowchatConfig,
    role: BridgeRole,
    session: Arc<AsyncMutex<Option<CowchatSession>>>,
}

struct CowchatSession {
    client: Arc<CowchatClient>,
    joined_rooms: BTreeSet<String>,
}

impl CowchatBackend {
    pub fn new(config: CowchatConfig) -> Self {
        Self::for_role(config, BridgeRole::Mcp)
    }

    pub fn for_role(config: CowchatConfig, role: BridgeRole) -> Self {
        Self {
            config,
            role,
            session: Arc::new(AsyncMutex::new(None)),
        }
    }

    async fn connect(&self) -> Result<CowchatClient, ServiceError> {
        self.config.validate_transport()?;
        // Backward-compatible remote credential: local UDS/loopback servers
        // accept an empty key, while deployments that require auth can keep
        // using the configured file.
        let key = std::fs::read_to_string(&self.config.api_key_file).unwrap_or_default();
        let key = key.trim();
        let role_agent_id = self.config.role_agent_id(self.role);
        let mut client = if let Some(socket) = &self.config.socket {
            CowchatClient::connect_uds(
                socket,
                key,
                &self.config.agent_name,
                Some(&role_agent_id),
                vec!["codex-wake".into()],
            )
            .await?
        } else if let Some(tcp) = &self.config.tcp {
            CowchatClient::connect_tcp(
                tcp,
                key,
                &self.config.agent_name,
                Some(&role_agent_id),
                vec!["codex-wake".into()],
            )
            .await?
        } else {
            return Err(ServiceError::InvalidCowchatTransport);
        };
        if let Some(env_name) = &self.config.room_key_env {
            let secret = std::env::var(env_name)
                .map_err(|_| ServiceError::MissingRoomKey(env_name.clone()))?;
            if secret.is_empty() {
                return Err(ServiceError::EmptyRoomKey(env_name.clone()));
            }
            client.set_room_secret(secret.as_bytes());
        }
        Ok(client)
    }

    pub(crate) async fn joined_client(
        &self,
        room: &str,
    ) -> Result<Arc<CowchatClient>, ServiceError> {
        let mut session = self.session.lock().await;
        if session.is_none() {
            *session = Some(CowchatSession {
                client: Arc::new(self.connect().await?),
                joined_rooms: BTreeSet::new(),
            });
        }
        let state = session.as_mut().expect("session initialized");
        if !state.joined_rooms.contains(room) {
            if let Err(error) = state.client.join_room(room).await {
                *session = None;
                return Err(ServiceError::Cowchat(error));
            }
            state.joined_rooms.insert(room.to_string());
        }
        Ok(state.client.clone())
    }

    pub(crate) async fn invalidate(&self) {
        *self.session.lock().await = None;
    }

    fn is_bridge_sender(&self, agent_id: &str) -> bool {
        [BridgeRole::Mcp, BridgeRole::Relay]
            .into_iter()
            .any(|role| self.config.role_agent_id(role) == agent_id)
    }

    async fn inspect_room_with_client(
        &self,
        room: &str,
        client: &CowchatClient,
    ) -> Result<RoomReadiness, ServiceError> {
        let info = client.room_info(room).await?;
        let ephemeral = info
            .pointer("/room/ephemeral")
            .and_then(Value::as_bool)
            .ok_or_else(|| ServiceError::InvalidRoomInfo(room.to_string()))?;
        let encrypted = info
            .pointer("/room/encrypted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tip = client.room_tip(room).await?;
        let key_validation = if !encrypted {
            "not_required".to_string()
        } else {
            let env_name = self
                .config
                .room_key_env
                .as_ref()
                .ok_or_else(|| ServiceError::EncryptedRoomNeedsKey(room.to_string()))?;
            let secret = std::env::var(env_name)
                .map_err(|_| ServiceError::MissingRoomKey(env_name.clone()))?;
            if secret.is_empty() {
                return Err(ServiceError::EmptyRoomKey(env_name.clone()));
            }
            if tip == 0 {
                "configured_unverified_empty_room".to_string()
            } else {
                let messages = client
                    .get_history_filtered(room, 1, None, None, Some(tip - 1))
                    .await?;
                if messages.is_empty() {
                    return Err(ServiceError::CannotValidateRoomKey(room.to_string()));
                }
                if messages
                    .iter()
                    .any(|message| message.content.starts_with(CIPHERTEXT_PREFIX))
                {
                    return Err(ServiceError::InvalidRoomKey(room.to_string()));
                }
                "verified_from_history".to_string()
            }
        };
        Ok(RoomReadiness {
            ephemeral,
            encrypted,
            key_validation,
            tip,
        })
    }
}

#[async_trait]
impl ChatBackend for CowchatBackend {
    async fn inspect_room(&self, room: &str) -> Result<RoomReadiness, ServiceError> {
        let client = self.joined_client(room).await?;
        match self.inspect_room_with_client(room, &client).await {
            Ok(readiness) => Ok(readiness),
            Err(error) => {
                if matches!(error, ServiceError::Cowchat(_)) {
                    self.invalidate().await;
                }
                Err(error)
            }
        }
    }

    async fn send_event(
        &self,
        target: &str,
        state_id: &str,
        room: &str,
        event: &WakeEvent,
        hint: WakeHint,
        event_digest: &str,
    ) -> Result<ChatMessage, ServiceError> {
        let client = self.joined_client(room).await?;
        let content = serde_json::to_string(event)?;
        let metadata = wake_metadata(target, state_id, event, hint, event_digest);
        let result = client
            .send_message_with_metadata(room, &content, None, Vec::new(), metadata)
            .await;
        if result.is_err() {
            self.invalidate().await;
        }
        Ok(result?)
    }

    async fn find_event(
        &self,
        target: &str,
        state_id: &str,
        room: &str,
        lookup: WakeEventLookup<'_>,
    ) -> Result<Option<ChatMessage>, ServiceError> {
        let client = self.joined_client(room).await?;
        let mut cursor = 0;
        loop {
            let messages = match client
                .get_history_filtered(room, 1000, None, None, Some(cursor))
                .await
            {
                Ok(messages) => messages,
                Err(error) => {
                    self.invalidate().await;
                    return Err(error.into());
                }
            };
            if messages.is_empty() {
                return Ok(None);
            }
            if let Some(message) = messages.iter().find(|message| {
                (self.is_bridge_sender(&message.agent_id)
                    || (lookup.allow_legacy_metadata && is_legacy_wake_for(message, target)))
                    && exact_wake_message(
                        message,
                        target,
                        state_id,
                        lookup.event,
                        lookup.hint,
                        lookup.event_digest,
                        lookup.allow_legacy_metadata,
                    )
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
        state_id: &str,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, ServiceError> {
        let client = self.joined_client(room).await?;
        let mut cursor = after_seq;
        let mut result = Vec::new();
        while result.len() < limit as usize {
            let messages = match client
                .get_history_filtered(room, 1000, None, None, Some(cursor))
                .await
            {
                Ok(messages) => messages,
                Err(error) => {
                    self.invalidate().await;
                    return Err(error.into());
                }
            };
            if messages.is_empty() {
                break;
            }
            cursor = messages.last().map(|message| message.seq).unwrap_or(cursor);
            for message in messages.iter().filter(|message| {
                (self.is_bridge_sender(&message.agent_id) && is_wake_for(message, target, state_id))
                    || is_legacy_wake_for(message, target)
            }) {
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
        self.wake_agent_for_expected_state(input, None).await
    }

    /// Relay scans carry the state generation they started in. They must not
    /// silently adopt a reset that happened while source history was in flight.
    pub(crate) async fn wake_agent_for_handle(
        &self,
        input: WakeAgentInput,
        expected: &TargetHandle,
    ) -> Result<WakeAgentOutput, ServiceError> {
        self.wake_agent_for_expected_state(input, Some(expected))
            .await
    }

    async fn wake_agent_for_expected_state(
        &self,
        input: WakeAgentInput,
        expected: Option<&TargetHandle>,
    ) -> Result<WakeAgentOutput, ServiceError> {
        validate_identifier("target", &input.target)?;
        validate_identifier("source", &input.source)?;
        validate_identifier("event_id", &input.event_id)?;
        validate_identifier("event_type", &input.event_type)?;
        let target = self.config.target(&input.target)?.clone();
        let identity = self.config.target_identity(&input.target)?;
        let handle = if let Some(expected) = expected {
            if expected.identity != identity
                || expected.alias != input.target
                || expected.room_id != target.room
            {
                return Err(ServiceError::TargetStateChanged {
                    expected: expected.state_id.clone(),
                    current: "configured-target-identity-changed".into(),
                });
            }
            let _target_lock = self
                .store
                .lock_target_exclusive_async(&input.target)
                .await?;
            // Any reset between the relay's room read and this call makes the
            // old handle stale before a Cowchat or Codex side effect.
            self.store.last_acked_seq(expected)?;
            expected.clone()
        } else {
            let room = self.chat.inspect_room(&target.room).await?;
            if room.ephemeral {
                return Err(ServiceError::EphemeralTargetRoom(target.room));
            }
            let _target_lock = self
                .store
                .lock_target_exclusive_async(&input.target)
                .await?;
            self.store
                .activate_target(&identity, &input.target, &target.room, room.tip)?
        };
        let request_json = serde_json::to_string(&json!({
            "specversion": "1.0",
            "id": &input.event_id,
            "source": &input.source,
            "type": &input.event_type,
            "subject": &input.subject,
            "time": &input.time,
            "data": &input.data,
        }))?;
        let event_time = input
            .time
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        chrono::DateTime::parse_from_rfc3339(&event_time)
            .map_err(|_| ServiceError::InvalidEventTime(event_time.clone()))?;
        let event = WakeEvent {
            specversion: "1.0".into(),
            id: input.event_id.clone(),
            source: input.source.clone(),
            event_type: input.event_type.clone(),
            subject: input.subject.clone(),
            time: event_time,
            data: input.data.clone(),
        };
        let event_json = serde_json::to_string(&event)?;
        if event_json.len() > MAX_EVENT_BYTES {
            return Err(ServiceError::EventTooLarge(event_json.len()));
        }

        let event_digest = format!("{:x}", Sha256::digest(event_json.as_bytes()));
        let now_unix = chrono::Utc::now().timestamp();
        let (reservation, delivery_claim) = {
            // Claim/takeover participates in the same fence used by the final
            // send. A newer generation therefore cannot be minted while an
            // older owner is committing its bounded delivery.
            let _target_lock = self
                .store
                .lock_target_exclusive_async(&input.target)
                .await?;
            self.store.last_acked_seq(&handle)?;
            let reservation = self.store.reserve_event(
                &handle,
                EventReservation {
                    source: &event.source,
                    event_id: &event.id,
                    request_json: &request_json,
                    event_json: &event_json,
                    event_digest: &event_digest,
                    room_id: &target.room,
                    wake_hint_rank: wake_hint_rank(input.wake_hint),
                    now_unix,
                },
            )?;
            let delivery_claim = self.store.claim_delivery(
                &handle,
                &event.source,
                &event.id,
                now_unix,
                self.config.codex.wake_lease_seconds,
            )?;
            (reservation, delivery_claim)
        };
        let event: WakeEvent = serde_json::from_str(&reservation.event_json)?;
        let seq = match delivery_claim {
            DeliveryClaim::Delivered(seq) => {
                let recovered = self
                    .chat
                    .find_event(
                        &input.target,
                        &handle.state_id,
                        &target.room,
                        WakeEventLookup {
                            event: &event,
                            hint: input.wake_hint,
                            event_digest: &reservation.event_digest,
                            allow_legacy_metadata: reservation.legacy_metadata,
                        },
                    )
                    .await?;
                match recovered {
                    Some(message) if message.seq == seq => seq,
                    Some(message) => {
                        return Err(ServiceError::RecoveredSequenceMismatch {
                            reserved: seq,
                            recovered: message.seq,
                        })
                    }
                    None => return Err(ServiceError::DeliveredEventMissing(seq)),
                }
            }
            DeliveryClaim::InFlight => return Err(StoreError::DeliveryInProgress.into()),
            DeliveryClaim::Claimed { generation } => {
                let delivery = async {
                    let recovered = self
                        .chat
                        .find_event(
                            &input.target,
                            &handle.state_id,
                            &target.room,
                            WakeEventLookup {
                                event: &event,
                                hint: input.wake_hint,
                                event_digest: &reservation.event_digest,
                                allow_legacy_metadata: reservation.legacy_metadata,
                            },
                        )
                        .await?;
                    // Historical recovery can span an unbounded number of
                    // pages and therefore runs without the filesystem fence.
                    // Reacquire it and renew the exact claim generation
                    // immediately before the bounded send and local commit.
                    let _target_lock = self
                        .store
                        .lock_target_exclusive_async(&input.target)
                        .await?;
                    self.store.last_acked_seq(&handle)?;
                    if !self.store.renew_delivery_claim(
                        &handle,
                        &event.source,
                        &event.id,
                        generation,
                        chrono::Utc::now().timestamp(),
                    )? {
                        return Err(StoreError::StaleDeliveryClaim.into());
                    }
                    let message = match recovered {
                        Some(message) => message,
                        None => {
                            self.chat
                                .send_event(
                                    &input.target,
                                    &handle.state_id,
                                    &target.room,
                                    &event,
                                    input.wake_hint,
                                    &reservation.event_digest,
                                )
                                .await?
                        }
                    };
                    self.store.mark_delivered(
                        &handle,
                        &event.source,
                        &event.id,
                        generation,
                        message.seq,
                        &message.message_id,
                    )?;
                    Ok::<ChatMessage, ServiceError>(message)
                };
                let delivery = self
                    .with_delivery_lease(&handle, &event.source, &event.id, generation, delivery)
                    .await;
                let message = match delivery {
                    Ok(message) => message,
                    Err(error) => {
                        // Reset may already have invalidated the handle. In
                        // that case the stale generation is already fenced and
                        // there is nothing current to release.
                        match self.store.release_delivery(
                            &handle,
                            &event.source,
                            &event.id,
                            generation,
                        ) {
                            Ok(_) | Err(StoreError::StaleTargetState { .. }) => {}
                            Err(store_error) => return Err(store_error.into()),
                        }
                        return Err(error);
                    }
                };
                message.seq
            }
        };

        if input.wake_hint < target.min_wake_hint {
            return Ok(WakeAgentOutput {
                accepted: true,
                duplicate: reservation.duplicate,
                target: input.target,
                state_id: handle.state_id,
                room: target.room,
                seq,
                wake: "filtered_by_recipient_policy".into(),
                codex: None,
            });
        }
        let codex = self
            .maybe_wake(&handle, &target.thread_id, seq, &event)
            .await?;
        Ok(WakeAgentOutput {
            accepted: true,
            duplicate: reservation.duplicate,
            target: input.target,
            state_id: handle.state_id,
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

    async fn with_delivery_lease<F>(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
        generation: i64,
        future: F,
    ) -> Result<ChatMessage, ServiceError>
    where
        F: Future<Output = Result<ChatMessage, ServiceError>>,
    {
        let renewed = self.store.renew_delivery_claim(
            handle,
            source,
            event_id,
            generation,
            chrono::Utc::now().timestamp(),
        )?;
        if !renewed {
            return Err(StoreError::StaleDeliveryClaim.into());
        }

        let lease_millis = u64::try_from(self.config.codex.wake_lease_seconds)
            .unwrap_or(1)
            .saturating_mul(1_000);
        let heartbeat = Duration::from_millis((lease_millis / 3).clamp(100, 5_000));
        let mut interval = tokio::time::interval(heartbeat);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        tokio::pin!(future);
        loop {
            tokio::select! {
                result = &mut future => return result,
                _ = interval.tick() => {
                    let renewed = self.store.renew_delivery_claim(
                        handle,
                        source,
                        event_id,
                        generation,
                        chrono::Utc::now().timestamp(),
                    )?;
                    if !renewed {
                        return Err(StoreError::StaleDeliveryClaim.into());
                    }
                }
            }
        }
    }

    pub async fn read_inbox(
        &self,
        input: WakeInboxReadInput,
    ) -> Result<WakeInboxReadOutput, ServiceError> {
        validate_identifier("target", &input.target)?;
        let target = self.config.target(&input.target)?.clone();
        let identity = self.config.target_identity(&input.target)?;
        let room = self.chat.inspect_room(&target.room).await?;
        if room.ephemeral {
            return Err(ServiceError::EphemeralTargetRoom(target.room));
        }
        let handle = {
            let _target_lock = self
                .store
                .lock_target_exclusive_async(&input.target)
                .await?;
            self.store
                .activate_target(&identity, &input.target, &target.room, room.tip)?
        };
        let after_cursor = match (&input.state_id, input.after_cursor) {
            (None, None) => self.store.last_acked_seq(&handle)?,
            (Some(state_id), Some(cursor)) if state_id == &handle.state_id => cursor,
            (Some(state_id), Some(_)) => {
                return Err(ServiceError::CursorStateChanged {
                    supplied: state_id.clone(),
                    current: handle.state_id,
                })
            }
            _ => return Err(ServiceError::IncompleteCursor),
        };
        if after_cursor < 0 {
            return Err(ServiceError::InvalidCursor(after_cursor));
        }
        self.store.authorize_read_cursor(&handle, after_cursor)?;
        let limit = input.limit.unwrap_or(100).clamp(1, MAX_READ_LIMIT);
        let messages = self
            .chat
            .read_events(
                &input.target,
                &handle.state_id,
                &target.room,
                after_cursor,
                limit,
            )
            .await?;
        let mut events = Vec::with_capacity(messages.len());
        for message in messages {
            let record = self
                .store
                .delivered_event(&handle, message.seq)?
                .ok_or(ServiceError::UnreservedWakeEvent(message.seq))?;
            let wake_hint = metadata_wake_hint(&message);
            let event: WakeEvent = serde_json::from_str(&message.content).map_err(|source| {
                ServiceError::InvalidStoredEvent {
                    seq: message.seq,
                    source,
                }
            })?;
            if message.content != record.event_json
                || wake_hint_rank(wake_hint) != record.wake_hint_rank
                || !exact_wake_message(
                    &message,
                    &input.target,
                    &handle.state_id,
                    &event,
                    wake_hint,
                    &record.event_digest,
                    record.legacy_metadata,
                )
            {
                return Err(ServiceError::WakeEventIntegrity(message.seq));
            }
            events.push(WakeInboxItem {
                seq: message.seq,
                message_id: message.message_id,
                sender: message.agent_name,
                event,
                wake_hint,
            });
        }
        let returned_cursors = events.iter().map(|event| event.seq).collect::<Vec<_>>();
        let highest_returned_seq = events.last().map(|event| event.seq).unwrap_or(after_cursor);
        let coverage_seq = events.last().map_or(room.tip, |event| event.seq);
        let expected_cursors =
            self.store
                .pending_seqs_through(&handle, after_cursor, coverage_seq)?;
        if expected_cursors != returned_cursors {
            return Err(ServiceError::IncompleteWakeHistory {
                expected: expected_cursors,
                returned: returned_cursors,
                through: coverage_seq,
            });
        }
        self.store.record_read(&handle, &returned_cursors)?;
        Ok(WakeInboxReadOutput {
            target: input.target,
            state_id: handle.state_id,
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
        validate_identifier("target", &input.target)?;
        let target = self.config.target(&input.target)?.clone();
        let identity = self.config.target_identity(&input.target)?;
        let room = self.chat.inspect_room(&target.room).await?;
        if room.ephemeral {
            return Err(ServiceError::EphemeralTargetRoom(target.room));
        }
        let handle = {
            let _target_lock = self
                .store
                .lock_target_exclusive_async(&input.target)
                .await?;
            self.store
                .activate_target(&identity, &input.target, &target.room, room.tip)?
        };
        if input.state_id != handle.state_id {
            return Err(ServiceError::CursorStateChanged {
                supplied: input.state_id,
                current: handle.state_id,
            });
        }
        let state = self.store.acknowledge(&handle, input.cursor)?;
        let mut followup_wake = None;
        if let Some(seq) = self
            .store
            .max_pending_eligible_seq(&handle, wake_hint_rank(target.min_wake_hint))?
        {
            if let Some(record) = self.store.delivered_event(&handle, seq)? {
                let event: WakeEvent = serde_json::from_str(&record.event_json)?;
                let outcome = self
                    .maybe_wake(&handle, &target.thread_id, seq, &event)
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
            state_id: handle.state_id,
            last_acked_seq: state.last_acked_seq,
            max_read_seq: state.max_read_seq,
            next_pending_seq: state.max_pending_seq,
            followup_wake,
        })
    }

    async fn maybe_wake(
        &self,
        handle: &TargetHandle,
        thread_id: &str,
        observed_seq: i64,
        event: &WakeEvent,
    ) -> Result<Option<CodexWakeOutcome>, ServiceError> {
        // App-server I/O is bounded by request_timeout_seconds. Holding the
        // target fence across that one actuator call prevents reset from
        // completing between the final generation check and the wake.
        let _target_lock = self
            .store
            .lock_target_exclusive_async(&handle.alias)
            .await?;
        self.store.last_acked_seq(handle)?;
        let Some(claim) = self.store.claim_wake(
            handle,
            observed_seq,
            chrono::Utc::now().timestamp(),
            self.config.codex.wake_lease_seconds,
        )?
        else {
            return Ok(None);
        };
        let reference = WakeReference {
            target: handle.alias.clone(),
            state_id: handle.state_id.clone(),
            room: handle.room_id.clone(),
            after_seq: self.store.last_acked_seq(handle)?,
            observed_seq,
            source: event.source.clone(),
            event_id: event.id.clone(),
            event_type: event.event_type.clone(),
        };
        match self.codex.wake(thread_id, &reference).await {
            Ok(outcome) => Ok(Some(outcome)),
            Err(error) => {
                self.store.release_wake(handle, claim)?;
                Err(ServiceError::AppServer(error))
            }
        }
    }
}

fn wake_metadata(
    target: &str,
    state_id: &str,
    event: &WakeEvent,
    hint: WakeHint,
    event_digest: &str,
) -> Value {
    json!({
        "kind": WAKE_KIND,
        "wake_target": target,
        "wake_state_id": state_id,
        "wake_source": event.source,
        "wake_event_id": event.id,
        "wake_event_type": event.event_type,
        "wake_hint": hint,
        "wake_digest": event_digest,
    })
}

fn exact_wake_message(
    message: &ChatMessage,
    target: &str,
    state_id: &str,
    event: &WakeEvent,
    hint: WakeHint,
    event_digest: &str,
    allow_legacy_metadata: bool,
) -> bool {
    let current_metadata = is_wake_for(message, target, state_id)
        && metadata_string(message, "wake_digest") == Some(event_digest);
    let legacy_metadata = allow_legacy_metadata && is_legacy_wake_for(message, target);
    message.content == serde_json::to_string(event).expect("wake event serializes")
        && (current_metadata || legacy_metadata)
        && metadata_string(message, "wake_source") == Some(event.source.as_str())
        && metadata_string(message, "wake_event_id") == Some(event.id.as_str())
        && metadata_string(message, "wake_event_type") == Some(event.event_type.as_str())
        && metadata_wake_hint(message) == hint
}

fn is_wake_for(message: &ChatMessage, target: &str, state_id: &str) -> bool {
    metadata_string(message, "kind") == Some(WAKE_KIND)
        && metadata_string(message, "wake_target") == Some(target)
        && metadata_string(message, "wake_state_id") == Some(state_id)
}

fn is_legacy_wake_for(message: &ChatMessage, target: &str) -> bool {
    metadata_string(message, "kind") == Some(WAKE_KIND)
        && metadata_string(message, "wake_target") == Some(target)
        && metadata_string(message, "wake_state_id").is_none()
        && metadata_string(message, "wake_digest").is_none()
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
    #[error("configure exactly one Cowchat transport")]
    InvalidCowchatTransport,
    #[error("environment variable {0} is required for the encrypted Cowchat room key")]
    MissingRoomKey(String),
    #[error("environment variable {0} is empty; encrypted Cowchat room keys must be non-empty")]
    EmptyRoomKey(String),
    #[error("Cowchat room {0:?} is encrypted but cowchat.room_key_env is not configured")]
    EncryptedRoomNeedsKey(String),
    #[error("configured key could not decrypt existing messages in encrypted Cowchat room {0:?}")]
    InvalidRoomKey(String),
    #[error("encrypted Cowchat room {0:?} has a nonzero tip but no readable latest message; cannot validate its key")]
    CannotValidateRoomKey(String),
    #[error("Cowchat room info for {0:?} omitted room.ephemeral")]
    InvalidRoomInfo(String),
    #[error("Cowchat room {0:?} is temporary; durable wake delivery requires a permanent room")]
    EphemeralTargetRoom(String),
    #[error("Cowchat wake message at seq {0} has no matching local reservation")]
    UnreservedWakeEvent(i64),
    #[error(
        "Cowchat wake message at seq {0} does not match its reserved content, digest, or policy"
    )]
    WakeEventIntegrity(i64),
    #[error("locally delivered Cowchat wake at seq {0} is missing or no longer matches its reserved envelope")]
    DeliveredEventMissing(i64),
    #[error(
        "recovered Cowchat wake sequence {recovered} does not match reserved sequence {reserved}"
    )]
    RecoveredSequenceMismatch { reserved: i64, recovered: i64 },
    #[error("wake event is {0} bytes; maximum is 262144")]
    EventTooLarge(usize),
    #[error("{0} must be non-empty, at most 512 characters, and contain no control characters")]
    InvalidIdentifier(&'static str),
    #[error("cursor must be non-negative, got {0}")]
    InvalidCursor(i64),
    #[error("state_id and after_cursor must either both be supplied or both be omitted")]
    IncompleteCursor,
    #[error("cursor belongs to stale target state {supplied:?}; current state is {current:?}")]
    CursorStateChanged { supplied: String, current: String },
    #[error(
        "target state changed from {expected:?} to {current:?} while relay history was in flight"
    )]
    TargetStateChanged { expected: String, current: String },
    #[error(
        "wake history is incomplete through room sequence {through}: expected local deliveries {expected:?}, returned {returned:?}"
    )]
    IncompleteWakeHistory {
        expected: Vec<i64>,
        returned: Vec<i64>,
        through: i64,
    },
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
    use crate::config::{CodexConfig, RelayConfig, TargetConfig};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    struct FakeChat {
        messages: Mutex<Vec<ChatMessage>>,
        ephemeral: bool,
        find_barrier: Option<Arc<tokio::sync::Barrier>>,
        find_started: Option<Arc<tokio::sync::Notify>>,
    }

    impl Default for FakeChat {
        fn default() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                ephemeral: false,
                find_barrier: None,
                find_started: None,
            }
        }
    }

    #[async_trait]
    impl ChatBackend for FakeChat {
        async fn inspect_room(&self, _room: &str) -> Result<RoomReadiness, ServiceError> {
            let tip = self
                .messages
                .lock()
                .unwrap()
                .last()
                .map_or(0, |message| message.seq);
            Ok(RoomReadiness {
                ephemeral: self.ephemeral,
                encrypted: false,
                key_validation: "not_required".into(),
                tip,
            })
        }

        async fn send_event(
            &self,
            target: &str,
            state_id: &str,
            room: &str,
            event: &WakeEvent,
            hint: WakeHint,
            event_digest: &str,
        ) -> Result<ChatMessage, ServiceError> {
            let mut messages = self.messages.lock().unwrap();
            let message = ChatMessage {
                message_id: format!("msg-{}", messages.len() + 1),
                room_id: room.into(),
                agent_id: "sender-id".into(),
                agent_name: "sender".into(),
                content: serde_json::to_string(event).unwrap(),
                reply_to_message: None,
                metadata: wake_metadata(target, state_id, event, hint, event_digest),
                timestamp: chrono::Utc::now(),
                seq: messages.len() as i64 + 1,
            };
            messages.push(message.clone());
            Ok(message)
        }

        async fn find_event(
            &self,
            target: &str,
            state_id: &str,
            _room: &str,
            lookup: WakeEventLookup<'_>,
        ) -> Result<Option<ChatMessage>, ServiceError> {
            let found = self
                .messages
                .lock()
                .unwrap()
                .iter()
                .find(|message| {
                    exact_wake_message(
                        message,
                        target,
                        state_id,
                        lookup.event,
                        lookup.hint,
                        lookup.event_digest,
                        lookup.allow_legacy_metadata,
                    )
                })
                .cloned();
            if let Some(started) = &self.find_started {
                started.notify_one();
            }
            if let Some(barrier) = &self.find_barrier {
                barrier.wait().await;
            }
            Ok(found)
        }

        async fn read_events(
            &self,
            target: &str,
            state_id: &str,
            _room: &str,
            after_seq: i64,
            limit: u32,
        ) -> Result<Vec<ChatMessage>, ServiceError> {
            Ok(self
                .messages
                .lock()
                .unwrap()
                .iter()
                .filter(|message| {
                    message.seq > after_seq
                        && (is_wake_for(message, target, state_id)
                            || is_legacy_wake_for(message, target))
                })
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
            relay: RelayConfig::default(),
            targets: BTreeMap::from([(
                "reviewer".into(),
                TargetConfig {
                    thread_id: "thr-1".into(),
                    room: "room".into(),
                    agent_id: None,
                    relay: false,
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
    async fn expired_delivery_owner_cannot_send_after_a_new_generation_claims() {
        let config = test_config(WakeHint::Normal);
        let identity = config.target_identity("reviewer").unwrap();
        let store = Arc::new(WakeStore::open_in_memory().unwrap());
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let started = Arc::new(tokio::sync::Notify::new());
        let chat = Arc::new(FakeChat {
            messages: Mutex::new(Vec::new()),
            ephemeral: false,
            find_barrier: Some(barrier),
            find_started: Some(started.clone()),
        });
        let service = WakeService::new(
            config,
            store.clone(),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );

        let first = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .wake_agent(wake_input("evt-expired-owner", WakeHint::Normal))
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        let handle = store
            .current_target(&identity, "reviewer", "room")
            .unwrap()
            .unwrap();
        store
            .expire_delivery_for_test(&handle, "ci", "evt-expired-owner")
            .unwrap();

        let second = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .wake_agent(wake_input("evt-expired-owner", WakeHint::Normal))
                    .await
            })
        };
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(2), second)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            first,
            Err(ServiceError::Store(StoreError::StaleDeliveryClaim))
        ));
        assert!(second.is_ok());
        assert_eq!(chat.messages.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retry_without_time_reuses_the_first_generated_event() {
        let chat = Arc::new(FakeChat::default());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        let mut input = wake_input("evt-generated-time", WakeHint::Normal);
        input.time = None;
        let first = service.wake_agent(input.clone()).await.unwrap();
        let duplicate = service.wake_agent(input).await.unwrap();
        assert!(!first.duplicate);
        assert!(duplicate.duplicate);
        assert_eq!(chat.messages.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_temporary_room_before_reserving_or_sending() {
        let chat = Arc::new(FakeChat {
            messages: Mutex::new(Vec::new()),
            ephemeral: true,
            find_barrier: None,
            find_started: None,
        });
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        assert!(matches!(
            service
                .wake_agent(wake_input("temporary", WakeHint::Normal))
                .await,
            Err(ServiceError::EphemeralTargetRoom(_))
        ));
        assert!(chat.messages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn delivered_retry_must_recover_the_exact_durable_envelope() {
        let chat = Arc::new(FakeChat::default());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        service
            .wake_agent(wake_input("disappeared", WakeHint::Normal))
            .await
            .unwrap();
        chat.messages.lock().unwrap().clear();
        assert!(matches!(
            service
                .wake_agent(wake_input("disappeared", WakeHint::Normal))
                .await,
            Err(ServiceError::Store(StoreError::RoomTipBehindCursor {
                max_delivered_seq: 1,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn arbitrary_empty_read_cursor_never_grants_ack_authority() {
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            Arc::new(FakeChat::default()),
            Arc::new(FakeWake::default()),
        );
        let current = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                state_id: None,
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            service
                .read_inbox(WakeInboxReadInput {
                    target: "reviewer".into(),
                    state_id: Some(current.state_id.clone()),
                    after_cursor: Some(i64::MAX),
                    limit: None,
                })
                .await,
            Err(ServiceError::Store(
                StoreError::UnauthorizedReadCursor { .. }
            ))
        ));
        assert!(service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                state_id: current.state_id,
                cursor: i64::MAX,
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn migrated_legacy_delivery_is_returned_and_acknowledgeable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let event = WakeEvent {
            specversion: "1.0".into(),
            id: "legacy-event".into(),
            source: "ci".into(),
            event_type: "build.completed".into(),
            subject: None,
            time: "2026-08-02T00:00:00Z".into(),
            data: json!({"status": "green"}),
        };
        let event_json = serde_json::to_string(&event).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE wake_events (
                     target TEXT NOT NULL, source TEXT NOT NULL, event_id TEXT NOT NULL,
                     event_json TEXT NOT NULL, room_id TEXT NOT NULL,
                     wake_hint_rank INTEGER NOT NULL, room_seq INTEGER, message_id TEXT,
                     created_at INTEGER NOT NULL,
                     PRIMARY KEY (target, source, event_id)
                 );
                 CREATE TABLE wake_target_state (
                     target TEXT PRIMARY KEY, last_acked_seq INTEGER NOT NULL DEFAULT 0,
                     max_read_seq INTEGER NOT NULL DEFAULT 0, wake_claimed_at INTEGER
                 );
                 INSERT INTO wake_target_state(target, last_acked_seq, max_read_seq)
                 VALUES ('reviewer', 6, 7);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO wake_events
                     (target, source, event_id, event_json, room_id, wake_hint_rank,
                      room_seq, message_id, created_at)
                 VALUES ('reviewer', 'ci', 'legacy-event', ?1, 'room', 1, 7, 'msg-7', 1)",
                [&event_json],
            )
            .unwrap();
        drop(connection);

        let config = test_config(WakeHint::Normal);
        let identity = config.target_identity("reviewer").unwrap();
        let store =
            Arc::new(WakeStore::open_for_legacy_maintenance(&path, &config.state_scope()).unwrap());
        let handle = store
            .migrate_legacy_target(&identity, "reviewer", "room", 7)
            .unwrap();
        let chat = Arc::new(FakeChat::default());
        chat.messages.lock().unwrap().push(ChatMessage {
            message_id: "msg-7".into(),
            room_id: "room".into(),
            agent_id: "legacy-bridge".into(),
            agent_name: "legacy bridge".into(),
            content: event_json,
            reply_to_message: None,
            metadata: json!({
                "kind": WAKE_KIND,
                "wake_target": "reviewer",
                "wake_source": "ci",
                "wake_event_id": "legacy-event",
                "wake_event_type": "build.completed",
                "wake_hint": WakeHint::Normal,
            }),
            timestamp: chrono::Utc::now(),
            seq: 7,
        });
        let service = WakeService::new(config, store, chat, Arc::new(FakeWake::default()));

        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                state_id: None,
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(inbox.state_id, handle.state_id);
        assert_eq!(inbox.events.len(), 1);
        assert_eq!(inbox.events[0].event, event);
        let acknowledged = service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                state_id: inbox.state_id,
                cursor: 7,
            })
            .await
            .unwrap();
        assert_eq!(acknowledged.last_acked_seq, 7);
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
                state_id: None,
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
        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                state_id: None,
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        let ack = service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                state_id: inbox.state_id,
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
        let delivered = service
            .wake_agent(wake_input("evt-1", WakeHint::Normal))
            .await
            .unwrap();
        assert!(service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                state_id: delivered.state_id,
                cursor: 1,
            })
            .await
            .is_err());
        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                state_id: None,
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        let ack = service
            .acknowledge(WakeInboxAckInput {
                target: "reviewer".into(),
                state_id: inbox.state_id,
                cursor: inbox.highest_returned_seq,
            })
            .await
            .unwrap();
        assert_eq!(ack.last_acked_seq, 1);
        assert_eq!(ack.next_pending_seq, None);
    }

    #[tokio::test]
    async fn reset_keeps_history_but_ignores_old_generation_and_rejects_stale_ack() {
        let config = test_config(WakeHint::Normal);
        let identity = config.target_identity("reviewer").unwrap();
        let store = Arc::new(WakeStore::open_in_memory().unwrap());
        let chat = Arc::new(FakeChat::default());
        let service = WakeService::new(
            config,
            store.clone(),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        let first = service
            .wake_agent(wake_input("evt-reset", WakeHint::Normal))
            .await
            .unwrap();
        let reset = store
            .reset_target(&identity, "reviewer", "room", first.seq)
            .unwrap();
        assert_ne!(first.state_id, reset.state_id);
        assert_eq!(chat.messages.lock().unwrap().len(), 1);

        let inbox = service
            .read_inbox(WakeInboxReadInput {
                target: "reviewer".into(),
                state_id: None,
                after_cursor: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(inbox.state_id, reset.state_id);
        assert!(inbox.events.is_empty());
        assert!(matches!(
            service
                .acknowledge(WakeInboxAckInput {
                    target: "reviewer".into(),
                    state_id: first.state_id,
                    cursor: first.seq,
                })
                .await,
            Err(ServiceError::CursorStateChanged { .. })
        ));

        let replay = service
            .wake_agent(wake_input("evt-reset", WakeHint::Normal))
            .await
            .unwrap();
        assert!(!replay.duplicate);
        assert_eq!(replay.state_id, reset.state_id);
        assert_eq!(replay.seq, 2);
    }

    #[tokio::test]
    async fn missing_lower_reserved_envelope_blocks_a_later_inbox_event() {
        let chat = Arc::new(FakeChat::default());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            Arc::new(WakeStore::open_in_memory().unwrap()),
            chat.clone(),
            Arc::new(FakeWake::default()),
        );
        service
            .wake_agent(wake_input("evt-lower", WakeHint::Normal))
            .await
            .unwrap();
        service
            .wake_agent(wake_input("evt-later", WakeHint::Normal))
            .await
            .unwrap();
        chat.messages
            .lock()
            .unwrap()
            .retain(|message| message.seq != 1);

        assert!(matches!(
            service
                .read_inbox(WakeInboxReadInput {
                    target: "reviewer".into(),
                    state_id: None,
                    after_cursor: None,
                    limit: None,
                })
                .await,
            Err(ServiceError::IncompleteWakeHistory {
                expected,
                returned,
                through: 2,
            }) if expected == vec![1, 2] && returned == vec![2]
        ));
    }

    #[tokio::test]
    async fn unknown_target_is_rejected_before_creating_a_lock_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let store = Arc::new(WakeStore::open(&path, "scope").unwrap());
        let service = WakeService::new(
            test_config(WakeHint::Normal),
            store,
            Arc::new(FakeChat::default()),
            Arc::new(FakeWake::default()),
        );
        let mut input = wake_input("unknown", WakeHint::Normal);
        input.target = "not-configured".into();
        assert!(matches!(
            service.wake_agent(input).await,
            Err(ServiceError::Config(ConfigError::UnknownTarget(_)))
        ));
        let lock_dir = path.with_file_name("wake.db.locks");
        assert_eq!(std::fs::read_dir(lock_dir).unwrap().count(), 0);
    }
}
