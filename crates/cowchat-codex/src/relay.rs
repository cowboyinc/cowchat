use crate::config::{BridgeConfig, ConfigError, TargetConfig, WakeHint};
use crate::service::{CowchatBackend, ServiceError, WakeAgentInput, WakeService};
use crate::store::{StoreError, WakeStore};
use async_trait::async_trait;
use cowchat_client::ClientError;
use cowchat_core::ChatMessage;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const RELAY_PAGE_SIZE: u32 = 500;
const WAKE_KIND: &str = "agent_wake";

#[async_trait]
pub trait RelayChatBackend: Send + Sync {
    async fn room_tip(&self, room: &str) -> Result<i64, RelayError>;
    async fn room_is_ephemeral(&self, room: &str) -> Result<bool, RelayError>;
    async fn read_messages(
        &self,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, RelayError>;
}

#[async_trait]
impl RelayChatBackend for CowchatBackend {
    async fn room_tip(&self, room: &str) -> Result<i64, RelayError> {
        let client = self.joined_client(room).await?;
        match client.room_tip(room).await {
            Ok(tip) => Ok(tip),
            Err(error) => {
                self.invalidate().await;
                Err(error.into())
            }
        }
    }

    async fn room_is_ephemeral(&self, room: &str) -> Result<bool, RelayError> {
        Ok(crate::service::ChatBackend::inspect_room(self, room)
            .await?
            .ephemeral)
    }

    async fn read_messages(
        &self,
        room: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, RelayError> {
        let client = self.joined_client(room).await?;
        match client
            .get_history_filtered(room, limit, None, None, Some(after_seq))
            .await
        {
            Ok(messages) => Ok(messages),
            Err(error) => {
                self.invalidate().await;
                Err(error.into())
            }
        }
    }
}

/// Managed, explicitly configured room-to-task relay. Ordinary Cowchat
/// messages stay the source of truth; the relay appends a thin, idempotent wake
/// envelope and invokes the existing Codex actuator.
pub struct WakeRelay {
    config: Arc<BridgeConfig>,
    store: Arc<WakeStore>,
    chat: Arc<dyn RelayChatBackend>,
    service: WakeService,
}

impl WakeRelay {
    pub fn new(
        config: BridgeConfig,
        store: Arc<WakeStore>,
        chat: Arc<dyn RelayChatBackend>,
        service: WakeService,
    ) -> Self {
        Self {
            config: Arc::new(config),
            store,
            chat,
            service,
        }
    }

    pub async fn run_forever(&self, from_start: bool) -> Result<(), RelayError> {
        let delay = Duration::from_millis(self.config.relay.poll_interval_ms);
        loop {
            match self.run_once(from_start).await {
                Ok(_) => {}
                Err(error) => {
                    // Cowchat and app-server are independent local services.
                    // Starting or restarting either must not permanently stop
                    // the managed listener; the durable cursors make retry safe.
                    log::error!("Cowchat Codex relay scan failed; retrying: {error}");
                }
            }
            tokio::time::sleep(delay).await;
        }
    }

    pub async fn run_once(&self, from_start: bool) -> Result<usize, RelayError> {
        let mut relayed = 0;
        let mut failures = Vec::new();
        for (alias, target) in self
            .config
            .targets
            .iter()
            .filter(|(_, target)| target.relay)
        {
            match self.scan_target(alias, target, from_start).await {
                Ok(count) => relayed += count,
                Err(error) => failures.push(TargetFailure {
                    target: alias.clone(),
                    permanent: error.is_permanent(),
                    error: error.to_string(),
                }),
            }
        }
        if !failures.is_empty() {
            return Err(RelayError::TargetFailures { relayed, failures });
        }
        Ok(relayed)
    }

    async fn scan_target(
        &self,
        alias: &str,
        target: &TargetConfig,
        from_start: bool,
    ) -> Result<usize, RelayError> {
        if self.chat.room_is_ephemeral(&target.room).await? {
            return Err(RelayError::EphemeralTarget {
                target: alias.to_string(),
                room: target.room.clone(),
            });
        }
        let room_tip = self.chat.room_tip(&target.room).await?;
        let identity = self.config.target_identity(alias)?;
        let handle = {
            let _target_lock = self.store.lock_target_exclusive_async(alias).await?;
            self.store
                .activate_target(&identity, alias, &target.room, room_tip)?
        };
        let initial_cursor = match self.store.relay_cursor(&handle)? {
            Some(cursor) if cursor <= room_tip => cursor,
            Some(cursor) => {
                return Err(RelayError::CursorAhead {
                    target: alias.to_string(),
                    room: target.room.clone(),
                    cursor,
                    room_tip,
                })
            }
            None if from_start => self.store.initialize_relay_cursor(&handle, 0)?,
            None => self.store.initialize_relay_cursor(&handle, room_tip)?,
        };
        let mut cursor = initial_cursor;
        let mut relayed = 0;
        loop {
            let mut messages = self
                .chat
                .read_messages(&target.room, cursor, RELAY_PAGE_SIZE)
                .await?;
            messages.sort_by_key(|message| message.seq);
            if messages.is_empty() {
                if cursor < room_tip {
                    return Err(RelayError::HistoryGap {
                        target: alias.to_string(),
                        room: target.room.clone(),
                        cursor,
                        room_tip,
                    });
                }
                break;
            }
            let mut previous_seq = cursor;
            for message in &messages {
                if message.seq <= previous_seq {
                    return Err(RelayError::NonMonotonicMessage {
                        target: alias.to_string(),
                        cursor: previous_seq,
                        seq: message.seq,
                    });
                }
                let expected_seq =
                    previous_seq
                        .checked_add(1)
                        .ok_or_else(|| RelayError::NonMonotonicMessage {
                            target: alias.to_string(),
                            cursor: previous_seq,
                            seq: message.seq,
                        })?;
                if message.seq != expected_seq {
                    return Err(RelayError::NonContiguousHistory {
                        target: alias.to_string(),
                        room: target.room.clone(),
                        expected: expected_seq,
                        found: message.seq,
                    });
                }
                previous_seq = message.seq;
            }
            let batch_len = messages.len();
            for message in messages {
                if message.seq <= cursor {
                    return Err(RelayError::NonMonotonicMessage {
                        target: alias.to_string(),
                        cursor,
                        seq: message.seq,
                    });
                }
                if should_relay(&message, target) {
                    let outcome = self
                        .service
                        .wake_agent_for_handle(relay_input(alias, &message), &handle)
                        .await?;
                    if !outcome.duplicate {
                        relayed += 1;
                    }
                    let acknowledged = self.store.last_acked_seq(&handle)? >= outcome.seq;
                    if outcome.wake != "filtered_by_recipient_policy" && !acknowledged {
                        // The envelope is durable, but the recipient has not
                        // completed it. Keep the original room cursor pinned so
                        // a crashed or non-acking turn is actuated again after
                        // the wake lease expires instead of going silent forever.
                        return Ok(relayed);
                    }
                }
                self.store.advance_relay_cursor(&handle, message.seq)?;
                cursor = message.seq;
            }
            if batch_len < RELAY_PAGE_SIZE as usize {
                if cursor < room_tip {
                    return Err(RelayError::HistoryGap {
                        target: alias.to_string(),
                        room: target.room.clone(),
                        cursor,
                        room_tip,
                    });
                }
                break;
            }
        }
        Ok(relayed)
    }
}

fn should_relay(message: &ChatMessage, target: &TargetConfig) -> bool {
    if target.agent_id.as_deref() == Some(message.agent_id.as_str()) {
        return false;
    }
    if message
        .metadata
        .get("kind")
        .and_then(|value| value.as_str())
        == Some(WAKE_KIND)
    {
        return false;
    }
    message
        .metadata
        .get("type")
        .and_then(|value| value.as_str())
        != Some("thinking")
}

fn relay_input(target: &str, message: &ChatMessage) -> WakeAgentInput {
    WakeAgentInput {
        target: target.to_string(),
        source: "cowchat.room-message".into(),
        event_id: message.message_id.clone(),
        event_type: "cowchat.message.received".into(),
        subject: Some(message.room_id.clone()),
        time: Some(message.timestamp.to_rfc3339()),
        data: json!({
            "room_id": message.room_id,
            "message_id": message.message_id,
            "seq": message.seq,
            "sender_agent_id": message.agent_id,
            "sender_agent_name": message.agent_name,
            "reply_to_message": message.reply_to_message,
        }),
        wake_hint: WakeHint::Normal,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error(
        "relay scan completed with {relayed} message(s) relayed and target failures: {failures:?}"
    )]
    TargetFailures {
        relayed: usize,
        failures: Vec<TargetFailure>,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(
        "relay target {target:?} received non-monotonic room sequence {seq} after cursor {cursor}"
    )]
    NonMonotonicMessage {
        target: String,
        cursor: i64,
        seq: i64,
    },
    #[error(
        "relay history for target {target:?} in room {room:?} is not contiguous: expected seq {expected}, found {found}"
    )]
    NonContiguousHistory {
        target: String,
        room: String,
        expected: i64,
        found: i64,
    },
    #[error("Cowchat room info for {0:?} omitted room.ephemeral")]
    InvalidRoomInfo(String),
    #[error(
        "relay target {target:?} uses temporary room {room:?}; ended-turn delivery requires a permanent room"
    )]
    EphemeralTarget { target: String, room: String },
    #[error(
        "relay cursor for target {target:?} in room {room:?} is ahead of the room tip ({cursor} > {room_tip}); the room or database may have been reset"
    )]
    CursorAhead {
        target: String,
        room: String,
        cursor: i64,
        room_tip: i64,
    },
    #[error(
        "relay history for target {target:?} in room {room:?} stopped at seq {cursor} before captured tip {room_tip}"
    )]
    HistoryGap {
        target: String,
        room: String,
        cursor: i64,
        room_tip: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFailure {
    pub target: String,
    pub permanent: bool,
    pub error: String,
}

impl RelayError {
    fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::TargetFailures { failures, .. } if failures.iter().all(|failure| failure.permanent)
        ) || matches!(
            self,
            Self::Config(_)
                | Self::NonMonotonicMessage { .. }
                | Self::NonContiguousHistory { .. }
                | Self::InvalidRoomInfo(_)
                | Self::EphemeralTarget { .. }
                | Self::CursorAhead { .. }
                | Self::HistoryGap { .. }
                | Self::Store(
                    StoreError::InvalidRelayCursor(_)
                        | StoreError::RelayRoomMismatch { .. }
                        | StoreError::MissingRelayState(_)
                        | StoreError::IdempotencyConflict { .. }
                        | StoreError::MissingReservation
                        | StoreError::AckBeyondRead { .. }
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::{AppServerError, CodexWakeOutcome, WakeBackend, WakeReference};
    use crate::config::{CodexConfig, CowchatConfig, RelayConfig};
    use crate::service::{ChatBackend, WakeEvent};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRoom {
        messages: Mutex<Vec<ChatMessage>>,
        ephemeral: bool,
        reported_tip: Option<i64>,
        transient_failures: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl RelayChatBackend for FakeRoom {
        async fn room_tip(&self, _room: &str) -> Result<i64, RelayError> {
            Ok(self.reported_tip.unwrap_or_else(|| {
                self.messages
                    .lock()
                    .unwrap()
                    .last()
                    .map(|message| message.seq)
                    .unwrap_or(0)
            }))
        }

        async fn room_is_ephemeral(&self, _room: &str) -> Result<bool, RelayError> {
            if self
                .transient_failures
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |remaining| remaining.checked_sub(1),
                )
                .is_ok()
            {
                return Err(RelayError::Client(ClientError::ConnectionClosed));
            }
            Ok(self.ephemeral)
        }

        async fn read_messages(
            &self,
            _room: &str,
            after_seq: i64,
            limit: u32,
        ) -> Result<Vec<ChatMessage>, RelayError> {
            Ok(self
                .messages
                .lock()
                .unwrap()
                .iter()
                .filter(|message| message.seq > after_seq)
                .take(limit as usize)
                .cloned()
                .collect())
        }
    }

    struct WakeMessages {
        messages: Mutex<Vec<ChatMessage>>,
        shared_room: Option<Arc<FakeRoom>>,
    }

    impl Default for WakeMessages {
        fn default() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                shared_room: None,
            }
        }
    }

    impl WakeMessages {
        fn for_room(room: Arc<FakeRoom>) -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                shared_room: Some(room),
            }
        }

        fn messages(&self) -> &Mutex<Vec<ChatMessage>> {
            self.shared_room
                .as_ref()
                .map_or(&self.messages, |room| &room.messages)
        }
    }

    #[async_trait]
    impl ChatBackend for WakeMessages {
        async fn inspect_room(
            &self,
            _room: &str,
        ) -> Result<crate::service::RoomReadiness, ServiceError> {
            let tip = self
                .messages()
                .lock()
                .unwrap()
                .last()
                .map_or(0, |message| message.seq);
            Ok(crate::service::RoomReadiness {
                ephemeral: false,
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
            let mut messages = self.messages().lock().unwrap();
            let seq = if self.shared_room.is_some() {
                messages.last().map_or(1, |message| message.seq + 1)
            } else {
                1000 + messages.len() as i64
            };
            let message = ChatMessage {
                message_id: format!("wake-{}", messages.len() + 1),
                room_id: room.into(),
                agent_id: "bridge".into(),
                agent_name: "bridge".into(),
                content: serde_json::to_string(event).unwrap(),
                reply_to_message: None,
                metadata: json!({
                    "kind": WAKE_KIND,
                    "wake_target": target,
                    "wake_state_id": state_id,
                    "wake_source": event.source,
                    "wake_event_id": event.id,
                    "wake_event_type": event.event_type,
                    "wake_hint": hint,
                    "wake_digest": event_digest,
                }),
                timestamp: Utc::now(),
                seq,
            };
            messages.push(message.clone());
            Ok(message)
        }

        async fn find_event(
            &self,
            target: &str,
            state_id: &str,
            _room: &str,
            lookup: crate::service::WakeEventLookup<'_>,
        ) -> Result<Option<ChatMessage>, ServiceError> {
            let expected_content = serde_json::to_string(lookup.event).unwrap();
            Ok(self
                .messages()
                .lock()
                .unwrap()
                .iter()
                .find(|message| {
                    message.content == expected_content
                        && message.metadata["wake_target"] == target
                        && message.metadata["wake_state_id"] == state_id
                        && message.metadata["wake_hint"] == json!(lookup.hint)
                        && message.metadata["wake_digest"] == lookup.event_digest
                })
                .cloned())
        }

        async fn read_events(
            &self,
            _target: &str,
            _state_id: &str,
            _room: &str,
            _after_seq: i64,
            _limit: u32,
        ) -> Result<Vec<ChatMessage>, ServiceError> {
            Ok(Vec::new())
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
                turn_id: "turn".into(),
            })
        }
    }

    fn message(seq: i64, agent_id: &str, metadata: serde_json::Value) -> ChatMessage {
        ChatMessage {
            message_id: format!("message-{seq}"),
            room_id: "room-id".into(),
            agent_id: agent_id.into(),
            agent_name: agent_id.into(),
            content: "payload stays in Cowchat".into(),
            reply_to_message: None,
            metadata,
            timestamp: Utc::now(),
            seq,
        }
    }

    fn test_relay(room: Arc<FakeRoom>) -> (WakeRelay, Arc<FakeWake>, Arc<WakeStore>) {
        let config = BridgeConfig {
            state_db: "unused".into(),
            cowchat: CowchatConfig::default(),
            codex: CodexConfig {
                app_server_endpoint: "ws://unused".into(),
                bearer_token_env: None,
                request_timeout_seconds: 1,
                wake_lease_seconds: 30,
            },
            relay: RelayConfig {
                poll_interval_ms: 1,
            },
            targets: BTreeMap::from([(
                "reviewer".into(),
                TargetConfig {
                    thread_id: "thread".into(),
                    room: "room-id".into(),
                    agent_id: Some("recipient".into()),
                    relay: true,
                    min_wake_hint: WakeHint::Normal,
                },
            )]),
        };
        let store = Arc::new(WakeStore::open_in_memory().unwrap());
        let wake = Arc::new(FakeWake::default());
        let service = WakeService::new(
            config.clone(),
            store.clone(),
            Arc::new(WakeMessages::for_room(room.clone())),
            wake.clone(),
        );
        (
            WakeRelay::new(config, store.clone(), room, service),
            wake,
            store,
        )
    }

    fn reviewer_handle(relay: &WakeRelay) -> Option<crate::store::TargetHandle> {
        let identity = relay.config.target_identity("reviewer").unwrap();
        relay
            .store
            .current_target(&identity, "reviewer", "room-id")
            .unwrap()
    }

    fn reviewer_cursor(relay: &WakeRelay) -> Option<i64> {
        reviewer_handle(relay).and_then(|handle| relay.store.relay_cursor(&handle).unwrap())
    }

    fn activate_reviewer(relay: &WakeRelay, tip: i64) -> crate::store::TargetHandle {
        let identity = relay.config.target_identity("reviewer").unwrap();
        relay
            .store
            .activate_target(&identity, "reviewer", "room-id", tip)
            .unwrap()
    }

    #[tokio::test]
    async fn first_run_seeds_tip_then_wakes_for_followup_and_resumes_from_cursor() {
        let room = Arc::new(FakeRoom::default());
        room.messages
            .lock()
            .unwrap()
            .push(message(1, "peer", json!({})));
        let (relay, wake, store) = test_relay(room.clone());
        assert_eq!(relay.run_once(false).await.unwrap(), 0);
        assert_eq!(reviewer_cursor(&relay), Some(1));

        room.messages
            .lock()
            .unwrap()
            .push(message(2, "peer", json!({})));
        assert_eq!(relay.run_once(false).await.unwrap(), 1);
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
        assert_eq!(reviewer_cursor(&relay), Some(1));
        assert_eq!(relay.run_once(false).await.unwrap(), 0);
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
        assert_eq!(reviewer_cursor(&relay), Some(1));

        // Expiry/release of an unacknowledged wake lease must actuate the same
        // durable event again; advancing the source cursor earlier would strand it.
        let handle = reviewer_handle(&relay).unwrap();
        store.expire_wake_for_test(&handle).unwrap();
        assert_eq!(relay.run_once(false).await.unwrap(), 0);
        assert_eq!(wake.calls.lock().unwrap().len(), 2);
        assert_eq!(reviewer_cursor(&relay), Some(1));

        let wake_seq = wake.calls.lock().unwrap()[0].observed_seq;
        store.record_read(&handle, &[wake_seq]).unwrap();
        store.acknowledge(&handle, wake_seq).unwrap();
        assert_eq!(relay.run_once(false).await.unwrap(), 0);
        assert_eq!(reviewer_cursor(&relay), Some(3));
    }

    #[tokio::test]
    async fn ignores_recipient_thinking_and_bridge_envelopes() {
        let room = Arc::new(FakeRoom::default());
        room.messages.lock().unwrap().extend([
            message(1, "recipient", json!({})),
            message(2, "peer", json!({"type": "thinking"})),
            message(3, "bridge", json!({"kind": WAKE_KIND})),
            message(4, "peer", json!({})),
        ]);
        let (relay, wake, _store) = test_relay(room);
        assert_eq!(relay.run_once(true).await.unwrap(), 1);
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
        assert_eq!(reviewer_cursor(&relay), Some(3));
    }

    #[tokio::test]
    async fn rejects_temporary_target_before_seeding_a_cursor() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(Vec::new()),
            ephemeral: true,
            reported_tip: None,
            transient_failures: 0.into(),
        });
        let (relay, wake, _store) = test_relay(room);
        let error = relay.run_once(false).await.unwrap_err();
        assert!(matches!(
            error,
            RelayError::TargetFailures { failures, .. }
                if failures.iter().any(|failure| failure.error.contains("temporary room"))
        ));
        assert!(wake.calls.lock().unwrap().is_empty());
        assert_eq!(reviewer_handle(&relay), None);
    }

    #[tokio::test]
    async fn rejects_cursor_ahead_after_room_database_reset() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(Vec::new()),
            ephemeral: false,
            reported_tip: Some(1),
            transient_failures: 0.into(),
        });
        let (relay, _wake, store) = test_relay(room);
        let handle = activate_reviewer(&relay, 1);
        store.initialize_relay_cursor(&handle, 2).unwrap();
        let error = relay.run_once(false).await.unwrap_err();
        assert!(matches!(
            error,
            RelayError::TargetFailures { failures, .. }
                if failures.iter().any(|failure| failure.error.contains("ahead of the room tip (2 > 1)"))
        ));
    }

    #[tokio::test]
    async fn fails_closed_when_captured_tip_is_missing_from_history() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(Vec::new()),
            ephemeral: false,
            reported_tip: Some(5),
            transient_failures: 0.into(),
        });
        let (relay, _wake, store) = test_relay(room);
        let handle = activate_reviewer(&relay, 5);
        store.initialize_relay_cursor(&handle, 0).unwrap();
        let error = relay.run_once(false).await.unwrap_err();
        assert!(matches!(
            error,
            RelayError::TargetFailures { failures, .. }
                if failures.iter().any(|failure| failure.error.contains("stopped at seq 0 before captured tip 5"))
        ));
    }

    #[tokio::test]
    async fn rejects_gap_before_first_returned_message_without_waking_or_advancing() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(vec![message(2, "peer", json!({}))]),
            ephemeral: false,
            reported_tip: Some(2),
            transient_failures: 0.into(),
        });
        let (relay, wake, _store) = test_relay(room);

        let error = relay.run_once(true).await.unwrap_err();

        assert!(matches!(
            error,
            RelayError::TargetFailures { failures, .. }
                if failures.iter().any(|failure| failure.error.contains("expected seq 1, found 2"))
        ));
        assert!(wake.calls.lock().unwrap().is_empty());
        assert_eq!(reviewer_cursor(&relay), Some(0));
    }

    #[tokio::test]
    async fn rejects_mid_page_gap_before_waking_or_advancing_any_row() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(vec![
                message(1, "recipient", json!({})),
                message(3, "peer", json!({})),
            ]),
            ephemeral: false,
            reported_tip: Some(3),
            transient_failures: 0.into(),
        });
        let (relay, wake, _store) = test_relay(room);

        let error = relay.run_once(true).await.unwrap_err();

        assert!(matches!(
            error,
            RelayError::TargetFailures { failures, .. }
                if failures.iter().any(|failure| failure.error.contains("expected seq 2, found 3"))
        ));
        assert!(wake.calls.lock().unwrap().is_empty());
        assert_eq!(reviewer_cursor(&relay), Some(0));
    }

    #[tokio::test]
    async fn advances_across_multiple_contiguous_pages() {
        let room = Arc::new(FakeRoom::default());
        room.messages.lock().unwrap().extend(
            (1..=i64::from(RELAY_PAGE_SIZE) + 1).map(|seq| message(seq, "recipient", json!({}))),
        );
        let (relay, wake, _store) = test_relay(room);

        assert_eq!(relay.run_once(true).await.unwrap(), 0);
        assert!(wake.calls.lock().unwrap().is_empty());
        assert_eq!(
            reviewer_cursor(&relay),
            Some(i64::from(RELAY_PAGE_SIZE) + 1)
        );
    }

    #[tokio::test]
    async fn managed_relay_retries_a_transient_startup_failure() {
        let room = Arc::new(FakeRoom {
            messages: Mutex::new(Vec::new()),
            ephemeral: false,
            reported_tip: Some(0),
            transient_failures: 1.into(),
        });
        let (relay, _wake, store) = test_relay(room);
        let identity = relay.config.target_identity("reviewer").unwrap();
        let task = tokio::spawn(async move { relay.run_forever(false).await });
        tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                if store
                    .current_target(&identity, "reviewer", "room-id")
                    .unwrap()
                    .is_some_and(|handle| store.relay_cursor(&handle).unwrap().is_some())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("relay should retry and seed its cursor after the transient failure");
        task.abort();
    }

    #[tokio::test]
    async fn broken_first_target_does_not_starve_later_healthy_target() {
        struct PartitionedRoom;

        #[async_trait]
        impl RelayChatBackend for PartitionedRoom {
            async fn room_tip(&self, room: &str) -> Result<i64, RelayError> {
                Ok(if room == "room-good" { 1 } else { 0 })
            }

            async fn room_is_ephemeral(&self, room: &str) -> Result<bool, RelayError> {
                Ok(room == "room-bad")
            }

            async fn read_messages(
                &self,
                room: &str,
                after_seq: i64,
                _limit: u32,
            ) -> Result<Vec<ChatMessage>, RelayError> {
                if room == "room-good" && after_seq == 0 {
                    let mut item = message(1, "peer", json!({}));
                    item.room_id = room.to_string();
                    Ok(vec![item])
                } else {
                    Ok(Vec::new())
                }
            }
        }

        let config = BridgeConfig {
            state_db: "unused".into(),
            cowchat: CowchatConfig::default(),
            codex: CodexConfig {
                app_server_endpoint: "ws://unused".into(),
                bearer_token_env: None,
                request_timeout_seconds: 1,
                wake_lease_seconds: 30,
            },
            relay: RelayConfig {
                poll_interval_ms: 1,
            },
            targets: BTreeMap::from([
                (
                    "a-broken".into(),
                    TargetConfig {
                        thread_id: "thread-a".into(),
                        room: "room-bad".into(),
                        agent_id: Some("recipient-a".into()),
                        relay: true,
                        min_wake_hint: WakeHint::Normal,
                    },
                ),
                (
                    "z-healthy".into(),
                    TargetConfig {
                        thread_id: "thread-z".into(),
                        room: "room-good".into(),
                        agent_id: Some("recipient-z".into()),
                        relay: true,
                        min_wake_hint: WakeHint::Normal,
                    },
                ),
            ]),
        };
        let store = Arc::new(WakeStore::open_in_memory().unwrap());
        let wake = Arc::new(FakeWake::default());
        let service = WakeService::new(
            config.clone(),
            store.clone(),
            Arc::new(WakeMessages::default()),
            wake.clone(),
        );
        let relay = WakeRelay::new(config, store, Arc::new(PartitionedRoom), service);
        let error = relay.run_once(true).await.unwrap_err();
        assert!(matches!(
            error,
            RelayError::TargetFailures { relayed: 1, failures }
                if failures.len() == 1 && failures[0].target == "a-broken"
        ));
        assert_eq!(wake.calls.lock().unwrap().len(), 1);
        assert_eq!(wake.calls.lock().unwrap()[0].target, "z-healthy");
    }
}
