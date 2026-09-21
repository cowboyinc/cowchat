//! Real CBQS WebSockets/RocksDB with fixture node authority; no live-chain claim.
#[cfg(feature = "cbfs-archive-test")]
#[path = "cbfs_archive_tests.rs"]
mod cbfs_archive_tests;
use super::cbqs::{CbqsOwnerLog, LogError};
use super::*;
use axum::{routing::get, Router};
use cbqs_client::{CheckpointTrustV2, SessionConfig};
use cbqsd::{
    activation::RuntimeCompatibility,
    store_v2::StoreV2,
    transport::{
        self,
        chain::ChainClient,
        connection::{BrokerCore, ProviderIdentity},
        TransportConfig,
    },
};
use cowboy_protocol_activation::{ActivationMetadata, Height, RuntimeSections};
use cowboy_protocol_codec::cbqs_v2 as wire;
use ed25519_dalek::{Signer, SigningKey};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

const INSTANCE: [u8; 32] = [0x11; 32];
const STREAM: [u8; 32] = [0x5A; 32];
const PROVIDER: [u8; 20] = [0xB2; 20];
const OWNER: [u8; 20] = [0xC1; 20];
const H_SNAPSHOT: u64 = 1_000;
fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}
fn key33(key: &SigningKey) -> [u8; 33] {
    let mut bytes = [1u8; 33];
    bytes[1..].copy_from_slice(key.verifying_key().as_bytes());
    bytes
}
fn now_ms() -> u64 {
    Utc::now().timestamp_millis() as u64
}
fn hex0x(bytes: &[u8]) -> String {
    format!(
        "0x{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

/// The §15 Defaults, as the node serialises them: decimal strings, because
/// several rows exceed what a JSON number carries exactly.
fn params() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("cbqs.snapshot_refresh_ms", "2000"),
        ("cbqs.max_snapshot_age_blocks", "300"),
        ("cbqs.max_clock_skew_ms", "60000"),
        ("cbqs.max_grant_ttl_ms", "86400000"),
        ("cbqs.session_handshake_timeout_ms", "10000"),
        ("cbqs.max_sessions_per_connection", "64"),
        ("cbqs.max_message_bytes", "262144"),
        ("cbqs.max_groups_per_stream", "4096"),
        ("cbqs.max_lanes_per_stream", "16384"),
        ("cbqs.max_lane_creates_per_min", "256"),
        ("cbqs.max_group_creates_per_min", "64"),
        ("cbqs.max_lane_list_page", "256"),
        ("cbqs.max_subscriptions_per_connection", "256"),
        ("cbqs.max_subscription_credit_bytes", "67108864"),
        ("cbqs.slow_consumer_timeout_ms", "60000"),
        // Small enough that a lone append's batch closes promptly — the whole
        // point of the per-stream commit ticker.
        ("cbqs.commit_batch_max_ms", "20"),
        ("cbqs.commit_batch_max_records", "1024"),
        ("cbqs.visibility_ms", "30000"),
        ("cbqs.max_attempts", "10"),
        ("cbqs.max_in_flight_per_group", "1024"),
        ("cbqs.retention_ms", "604800000"),
        ("cbqs.max_retained_bytes_per_stream", "1073741824"),
        ("cbqs.max_append_bytes_per_sec", "1048576"),
        ("cbqs.max_delivered_bytes_per_sec", "2097152"),
    ])
}

/// The node's `/cbqs/snapshot/{stream_id}` body, field for field.
fn snapshot_json(admin_key: [u8; 33], provider_key: [u8; 33]) -> String {
    let params: BTreeMap<&str, &str> = params();
    serde_json::json!({
        "h_snapshot": H_SNAPSHOT,
        "chain_instance_id": hex0x(&INSTANCE),
        "stream": {
            "stream_id": hex0x(&STREAM),
            "owner": hex0x(&OWNER),
            "owner_nonce": 7,
            "provider": hex0x(&PROVIDER),
            "admin_key": hex0x(&admin_key),
            "authorization_generation": 2,
            "status": "Active",
        },
        "provider": {
            "provider": hex0x(&PROVIDER),
            "signing_key": hex0x(&provider_key),
            "signing_key_since": 1,
            "previous_signing_key": serde_json::Value::Null,
            "previous_signing_key_since": 0,
            "endpoints": ["wss://cbqs.example/ws"],
            "accepts_new": true,
            "max_streams": 64,
            "assigned_streams": 1,
            "metadata_hash": hex0x(&[0u8; 32]),
        },
        // §6.2: serviceable — 1000 blocks elapsed at rate 1 for one stream is
        // 1000 due, and the balance covers it many times over.
        "account": {
            "owner": hex0x(&OWNER),
            "provider": hex0x(&PROVIDER),
            "balance": "1000000000000",
            "active_streams": 1,
            "rate_per_block": "1",
            "last_settled_block": 0,
            "suspended": false,
        },
        "params": params,
    })
    .to_string()
}

struct Fixture {
    broker: SocketAddr,
    core: BrokerCore,
    serving: tokio::task::JoinHandle<()>,
    node: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.core.shutdown_handle().shut_down();
        self.serving.abort();
        self.node.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let body = snapshot_json(key33(&key(0xA1)), key33(&key(0xB2)));
        // Fixture authority only: pin the expectation before serving metadata.
        let metadata = ActivationMetadata::new(
            1,
            RuntimeSections {
                heights: BTreeMap::from([("TEST_ACTIVATION_HEIGHT".into(), Height(0))]),
                policy: Default::default(),
                governance: Default::default(),
            },
        )
        .unwrap();
        let activation = RuntimeCompatibility::new(Some(metadata.runtime_fingerprint.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let node = tokio::spawn(async move {
            let app = Router::new()
                .route(
                    "/cbqs/snapshot/{stream_id}",
                    get(move || {
                        let body = body.clone();
                        async move { body }
                    }),
                )
                .route(
                    "/chain-info",
                    get(move || {
                        let metadata = metadata.clone();
                        async move { axum::Json(serde_json::json!({ "activation": metadata })) }
                    }),
                );
            axum::serve(listener, app).await.unwrap();
        });
        let node_url = format!("http://{address}");
        activation.probe(&node_url).await;
        activation.check().unwrap();
        activation.spawn(node_url);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StoreV2::open(directory.path(), INSTANCE).unwrap());
        let (wakeup, _) = tokio::sync::broadcast::channel(64);
        let core = BrokerCore::new(
            store,
            ChainClient::new(format!("http://{address}"), INSTANCE),
            Arc::new(ProviderIdentity {
                address: cowboy_protocol_codec::Address::from_bytes(PROVIDER),
                signing_key: key(0xB2),
            }),
            wakeup,
        )
        .with_activation(activation);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker = listener.local_addr().unwrap();
        let mut stop = core.shutdown_signal();
        let served = core.clone();
        let serving = tokio::spawn(async move {
            transport::serve(
                listener,
                served,
                TransportConfig {
                    handshake_idle_ms: 0,
                    retention_sweep_ms: 0,
                    ..TransportConfig::default()
                },
                async move {
                    let _ = stop.changed().await;
                },
            )
            .await
            .unwrap();
        });
        Self {
            broker,
            core,
            serving,
            node,
            _directory: directory,
        }
    }
    fn config(&self, epoch: u64) -> SessionConfig {
        let admin = key(0xA1);
        let holder = key(0xC3);
        let provider = wire::ProviderRecordV2 {
            version: wire::CBQS_VERSION_V2,
            chain_instance_id: INSTANCE,
            provider: cowboy_protocol_codec::Address::from_bytes(PROVIDER),
            signing_key: wire::SigningPublicKeyV2 {
                algorithm: wire::SigningKeyAlgorithmV2::Ed25519,
                key_bytes: key(0xB2).verifying_key().to_bytes(),
            },
            signing_key_since: 1,
            previous_signing_key: None,
            previous_signing_key_since: 0,
            endpoints: Vec::new(),
            accepts_new: true,
            max_streams: 64,
            assigned_streams: 1,
            metadata_hash: [0; 32],
        };
        let mut grant = wire::StreamGrantV2 {
            version: wire::CBQS_VERSION_V2,
            chain_instance_id: INSTANCE,
            stream_id: STREAM,
            authorization_generation: 2,
            policy_epoch: epoch,
            grant_nonce: [0x77; 32],
            holder_signing_key: wire::SigningPublicKeyV2 {
                algorithm: wire::SigningKeyAlgorithmV2::Ed25519,
                key_bytes: holder.verifying_key().to_bytes(),
            },
            verbs: wire::CBQS_V2_VERB_APPEND
                | wire::CBQS_V2_VERB_REPLAY
                | wire::CBQS_V2_VERB_CONSUME
                | wire::CBQS_V2_VERB_LANE_ADMIN,
            lane_scope: wire::LaneScopeV2::Any,
            group_scope: wire::GroupScopeV2::Any,
            not_before_ms: now_ms().saturating_sub(60_000),
            expires_at_ms: now_ms() + 3_600_000,
            max_message_bytes: 262_144,
            max_append_bytes_per_sec: 1_048_576,
            signature: wire::CbqsSignatureV2([0; 64]),
        };
        grant.signature = wire::CbqsSignatureV2(
            admin
                .sign(&cowboy_protocol_codec::keccak256(
                    &wire::stream_grant_signing_bytes_v2(&grant),
                ))
                .to_bytes(),
        );
        SessionConfig {
            broker_url: format!("ws://{}/ws", self.broker),
            grant,
            holder,
            handshake_timeout: Duration::from_secs(15),
            checkpoints: CheckpointTrustV2::ChainProvider(Box::new(provider)),
        }
    }
    async fn connect(&self, epoch: u64) -> Result<CbqsOwnerLog, LogError> {
        self.connect_config(self.config(epoch)).await
    }
    async fn connect_config(&self, config: SessionConfig) -> Result<CbqsOwnerLog, LogError> {
        let fence = cbqs_client::connect_socket(&config.broker_url)
            .await
            .unwrap();
        let session = cbqs_client::connect_socket(&config.broker_url)
            .await
            .unwrap();
        CbqsOwnerLog::attach(
            fence,
            session,
            config,
            |bytes| {
                key(0xA1)
                    .sign(&cowboy_protocol_codec::keccak256(bytes))
                    .to_bytes()
            },
            now_ms(),
        )
        .await
    }
}

#[tokio::test]
async fn fence_does_not_allocate_exclusive_ownership_between_same_epoch_holders() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let mut config = fixture.config(1);
    config.holder = key(0xD4);
    config.grant.holder_signing_key.key_bytes = config.holder.verifying_key().to_bytes();
    config.grant.grant_nonce = [0x88; 32];
    config.grant.signature = wire::CbqsSignatureV2(
        key(0xA1)
            .sign(&cowboy_protocol_codec::keccak256(
                &wire::stream_grant_signing_bytes_v2(&config.grant),
            ))
            .to_bytes(),
    );
    // A different authorized holder fences the SAME epoch successfully.
    let mut second = fixture.connect_config(config).await.unwrap();
    let one = first.room_lane("owner-a", "one").await.unwrap();
    let two = second.room_lane("owner-a", "two").await.unwrap();
    let create_one = super::tests::create("one", one);
    let create_two = super::tests::create("two", two);
    let (left, right) = tokio::join!(first.append(0, &create_one), second.append(0, &create_two));
    let mut sequences = [left.unwrap(), right.unwrap()];
    sequences.sort();
    assert_eq!(
        sequences,
        [1, 2],
        "a separate unique-epoch allocation step is required"
    );
}

#[tokio::test]
async fn actual_broker_replays_interleaved_room_lanes_and_fences_the_previous_owner() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    assert!(first
        .replay_from_start(0, 100, 100_000)
        .await
        .unwrap()
        .is_empty());
    let one = first.room_lane("owner-a", "one").await.unwrap();
    assert_eq!(first.room_lane("owner-a", "one").await.unwrap(), one);
    let two = first.room_lane("owner-a", "two").await.unwrap();
    assert_ne!(one, two);
    let entries = [
        (0, super::tests::create("one", one)),
        (0, super::tests::create("two", two)),
        (one, super::tests::message("a", "one")),
        (two, super::tests::message("b", "two")),
        (one, super::tests::message("a", "one")),
    ];
    let mut live = OwnerState::new("owner-a".into());
    for (lane, command) in &entries {
        let sequence = first.append(*lane, command).await.unwrap();
        live.apply(sequence, *lane, command).unwrap();
    }
    let mut next = fixture.connect(2).await.unwrap();
    assert_eq!(next.epoch(), 2);
    assert!(matches!(
        first
            .append(one, &super::tests::message("late", "one"))
            .await,
        Err(LogError::Fenced)
    ));
    let records = next.replay_from_start(5, 100, 100_000).await.unwrap();
    assert_eq!(records.len(), 5);
    let mut rebuilt = OwnerState::new("owner-a".into());
    for record in records {
        rebuilt
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(
        serde_json::to_value(live).unwrap(),
        serde_json::to_value(&rebuilt).unwrap()
    );
    assert_eq!(rebuilt.room("one").unwrap().messages.len(), 1);
    assert_eq!(fixture.core.store.tail(&STREAM).unwrap(), 5);
    assert!(
        matches!(fixture.connect(1).await, Err(LogError::Fenced)),
        "an old allocated epoch cannot adopt a newer broker floor"
    );
}

#[tokio::test]
async fn expired_history_is_not_treated_as_a_fresh_stream() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    first
        .append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 2)
        .unwrap();
    assert_eq!(fixture.core.store.tail(&STREAM).unwrap(), 1);
    let mut next = fixture.connect(2).await.unwrap();
    assert!(matches!(
        next.replay_from_start(0, 100, 100_000).await,
        Err(LogError::HistoryGap)
    ));
    assert!(
        matches!(
            next.append(0, &super::tests::create("two", 2)).await,
            Err(LogError::Unavailable)
        ),
        "failed recovery cannot become a fresh writable room"
    );
}

#[tokio::test]
async fn replay_bounds_and_known_checkpoint_rollback_fail_closed() {
    let fixture = Fixture::new().await;
    let mut log = fixture.connect(1).await.unwrap();
    assert!(matches!(
        log.replay_from_start(1, 100, 100_000).await,
        Err(LogError::HistoryGap)
    ));
    let mut log = fixture.connect(2).await.unwrap();
    let lane = log.room_lane("owner-a", "one").await.unwrap();
    log.append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    assert!(matches!(
        log.replay_from_start(1, 100, 1).await,
        Err(LogError::ReplayLimit)
    ));
    assert!(matches!(
        log.append(0, &super::tests::create("two", 2)).await,
        Err(LogError::Unavailable)
    ));
}

#[tokio::test]
async fn replay_replenishes_credit_without_skipping_records() {
    let fixture = Fixture::new().await;
    let mut log = fixture.connect(1).await.unwrap();
    let lane = log.room_lane("owner-a", "one").await.unwrap();
    log.append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    let mut expected = Vec::new();
    for n in 0..10 {
        let mut command = super::tests::message(&format!("large-{n}"), "one");
        let CommandBody::AppendMessage { ciphertext, .. } = &mut command.body else {
            unreachable!()
        };
        *ciphertext = format!("cow1:{}", "a".repeat(130_000));
        log.append(lane, &command).await.unwrap();
        expected.push(command);
        // Stay below the real broker's 1 MiB/s append budget. Replay itself
        // must cross the adapter's 1 MiB credit window without any test grants.
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let mut next = fixture.connect(2).await.unwrap();
    let records = next.replay_from_start(11, 100, 2_000_000).await.unwrap();
    assert_eq!(records.len(), 11);
    for (record, expected) in records[1..].iter().zip(expected) {
        assert_eq!(
            serde_json::to_value(&record.command).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[tokio::test]
async fn replay_rejects_payload_changed_after_broker_commit() {
    use commonware_codec::{DecodeExt, Encode};
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    first
        .append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    first
        .append(lane, &super::tests::message("a", "one"))
        .await
        .unwrap();

    // Relay a real session but change one payload byte on delivery. The JSON
    // stays valid and the signed checkpoint is untouched: recovery must verify
    // content, not just trust the authenticated session or checkpoint chain.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let broker_url = format!("ws://{}/ws", fixture.broker);
    let proxy = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut downstream = tokio_tungstenite::accept_async(socket).await.unwrap();
        let (mut upstream, _) = tokio_tungstenite::connect_async(broker_url).await.unwrap();
        loop {
            tokio::select! {
                frame = downstream.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    if upstream.send(frame).await.is_err() { break }
                }
                frame = upstream.next() => {
                    let Some(Ok(mut frame)) = frame else { break };
                    if let Message::Binary(bytes) = &frame {
                        if let Ok(mut decoded) = wire::CbqsServerFrameV2::decode(bytes.as_ref()) {
                            if let wire::CbqsServerFrameV2::Delivery { payload, .. } = &mut decoded {
                                let mut command: Command = serde_json::from_slice(payload).unwrap();
                                if let CommandBody::AppendMessage { ciphertext, .. } = &mut command.body {
                                    *ciphertext = "cow1:tampered-ciphertext".into();
                                    *payload = serde_json::to_vec(&command).unwrap();
                                    frame = Message::Binary(decoded.encode());
                                }
                            }
                        }
                    }
                    if downstream.send(frame).await.is_err() { break }
                }
            }
        }
    });
    let config = fixture.config(2);
    let fence = cbqs_client::connect_socket(&config.broker_url)
        .await
        .unwrap();
    let session = cbqs_client::connect_socket(&format!("ws://{proxy_addr}/ws"))
        .await
        .unwrap();
    let mut next = CbqsOwnerLog::attach(
        fence,
        session,
        config,
        |bytes| {
            key(0xA1)
                .sign(&cowboy_protocol_codec::keccak256(bytes))
                .to_bytes()
        },
        now_ms(),
    )
    .await
    .unwrap();
    let result = next.replay_from_start(2, 100, 100_000).await;
    proxy.abort();
    assert!(matches!(result, Err(LogError::Verification)), "{result:?}");
    assert!(matches!(
        next.append(lane, &super::tests::message("b", "one")).await,
        Err(LogError::Unavailable)
    ));
}

#[tokio::test]
async fn verified_boundary_resumes_after_retention_with_suffix_sized_limits() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let one = first.room_lane("owner-a", "one").await.unwrap();
    let two = first.room_lane("owner-a", "two").await.unwrap();
    first
        .append(0, &super::tests::create("one", one))
        .await
        .unwrap();
    first
        .append(0, &super::tests::create("two", two))
        .await
        .unwrap();
    first
        .append(one, &super::tests::message("a", "one"))
        .await
        .unwrap();
    let prefix = first.replay(None, 3, 100, 100_000).await.unwrap();
    let checkpoint = prefix.checkpoint.unwrap();
    assert_eq!(checkpoint.sequence(), 3);
    let mut rebuilt = OwnerState::new("owner-a".into());
    for record in prefix.records {
        rebuilt
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    first
        .append(two, &super::tests::message("b", "two"))
        .await
        .unwrap();
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 4)
        .unwrap();
    let mut next = fixture.connect(2).await.unwrap();
    // Only one new record, even though the stream has four and three lanes.
    let suffix = next.replay(Some(&checkpoint), 4, 1, 100_000).await.unwrap();
    assert_eq!(suffix.records.len(), 1);
    assert_eq!(suffix.records[0].sequence, 4);
    for record in suffix.records {
        rebuilt
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(rebuilt.applied_through(), 4);
    assert_eq!(rebuilt.room("one").unwrap().messages.len(), 1);
    assert_eq!(rebuilt.room("two").unwrap().messages.len(), 1);
    let checkpoint = suffix.checkpoint.unwrap();
    assert_eq!(checkpoint.sequence(), 4);
    // An already held and verified complete prefix stays usable when CBQS
    // expires all records. This never manufactures an empty owner state.
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 5)
        .unwrap();
    let empty = next.replay(Some(&checkpoint), 4, 1, 100_000).await.unwrap();
    assert!(empty.records.is_empty());
    assert_eq!(empty.checkpoint.unwrap().sequence(), 4);
}

#[tokio::test]
async fn checkpoint_cannot_bridge_a_missing_unarchived_suffix() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    first
        .append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    let checkpoint = first
        .replay(None, 1, 10, 100_000)
        .await
        .unwrap()
        .checkpoint
        .unwrap();
    first
        .append(lane, &super::tests::message("a", "one"))
        .await
        .unwrap();
    first
        .append(lane, &super::tests::message("b", "one"))
        .await
        .unwrap();
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 3)
        .unwrap();
    let mut next = fixture.connect(2).await.unwrap();
    assert!(matches!(
        next.replay(Some(&checkpoint), 3, 10, 100_000).await,
        Err(LogError::HistoryGap)
    ));
    assert!(matches!(
        next.append(lane, &super::tests::message("c", "one")).await,
        Err(LogError::Unavailable)
    ));
}

#[tokio::test]
async fn archived_wire_records_restore_after_full_retention_then_join_live_suffix() {
    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    first
        .append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    first
        .append(lane, &super::tests::message("a", "one"))
        .await
        .unwrap();
    let prefix = first.replay(None, 2, 10, 100_000).await.unwrap();
    let archive = prefix.archive_bytes(100_000).unwrap().unwrap();
    drop(prefix);
    drop(first);
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 3)
        .unwrap();

    // No prior in-memory checkpoint remains. Only archived wire bytes and the
    // fresh session's trusted provider authority can rebuild the prefix.
    let mut next = fixture.connect(2).await.unwrap();
    let restored = next.restore_archive(&archive, None, 10, 100_000).unwrap();
    let mut state = OwnerState::new("owner-a".into());
    for record in restored.records {
        state
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(state.room("one").unwrap().messages.len(), 1);
    let checkpoint = restored.checkpoint.unwrap();
    assert_eq!(checkpoint.sequence(), 2);
    next.append(lane, &super::tests::message("b", "one"))
        .await
        .unwrap();
    let suffix = next.replay(Some(&checkpoint), 3, 1, 100_000).await.unwrap();
    let suffix_archive = suffix.archive_bytes(100_000).unwrap().unwrap();
    let recovered_suffix = next
        .restore_archive(&suffix_archive, Some(&checkpoint), 1, 100_000)
        .unwrap();
    assert_eq!(recovered_suffix.records.len(), 1);
    for record in recovered_suffix.records {
        state
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(state.applied_through(), 3);
    assert_eq!(state.room("one").unwrap().messages.len(), 2);
}

#[tokio::test]
async fn archived_bytes_cannot_forge_or_truncate_a_verified_boundary() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let fixture = Fixture::new().await;
    let mut first = fixture.connect(1).await.unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    first
        .append(0, &super::tests::create("one", lane))
        .await
        .unwrap();
    first
        .append(lane, &super::tests::message("a", "one"))
        .await
        .unwrap();
    let replay = first.replay(None, 2, 10, 100_000).await.unwrap();
    assert!(matches!(
        replay.archive_bytes(1),
        Err(LogError::ReplayLimit)
    ));
    let bytes = replay.archive_bytes(100_000).unwrap().unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut forged = original.clone();
    forged["records"][1]["payload"] = serde_json::json!(STANDARD.encode(b"{}"));
    let mut truncated = original.clone();
    truncated["records"].as_array_mut().unwrap().pop();
    let mut reordered = original.clone();
    reordered["records"].as_array_mut().unwrap().swap(0, 1);
    for (index, invalid) in [forged, truncated, reordered].into_iter().enumerate() {
        let mut next = fixture.connect(index as u64 + 2).await.unwrap();
        let result =
            next.restore_archive(&serde_json::to_vec(&invalid).unwrap(), None, 10, 100_000);
        assert!(
            matches!(result, Err(LogError::Verification | LogError::HistoryGap)),
            "{result:?}"
        );
        assert!(matches!(
            next.append(lane, &super::tests::message("b", "one")).await,
            Err(LogError::Unavailable)
        ));
    }
    let mut next = fixture.connect(5).await.unwrap();
    assert!(matches!(
        next.restore_archive(&bytes, None, 10, 1),
        Err(LogError::ReplayLimit)
    ));
}

#[tokio::test]
async fn cancelling_an_append_after_commit_retires_the_uncertain_session() {
    use commonware_codec::DecodeExt;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let fixture = Fixture::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let broker_url = format!("ws://{}/ws", fixture.broker);
    let held = Arc::new(tokio::sync::Notify::new());
    let observed = held.clone();
    let proxy = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut downstream = tokio_tungstenite::accept_async(socket).await.unwrap();
        let (mut upstream, _) = tokio_tungstenite::connect_async(broker_url).await.unwrap();
        loop {
            tokio::select! {
                frame = downstream.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    if upstream.send(frame).await.is_err() { break }
                }
                frame = upstream.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    if let Message::Binary(bytes) = &frame {
                        if matches!(wire::CbqsServerFrameV2::decode(bytes.as_ref()),
                            Ok(wire::CbqsServerFrameV2::Response { body: wire::CbqsResponseBodyV2::Appended { .. }, .. })) {
                            observed.notify_one();
                            std::future::pending::<()>().await;
                        }
                    }
                    if downstream.send(frame).await.is_err() { break }
                }
            }
        }
    });
    let config = fixture.config(1);
    let fence = cbqs_client::connect_socket(&config.broker_url)
        .await
        .unwrap();
    let session = cbqs_client::connect_socket(&format!("ws://{proxy_addr}/ws"))
        .await
        .unwrap();
    let mut first = CbqsOwnerLog::attach(
        fence,
        session,
        config,
        |bytes| {
            key(0xA1)
                .sign(&cowboy_protocol_codec::keccak256(bytes))
                .to_bytes()
        },
        now_ms(),
    )
    .await
    .unwrap();
    let lane = first.room_lane("owner-a", "one").await.unwrap();
    let command = super::tests::create("one", lane);
    {
        let append = first.append(0, &command);
        tokio::pin!(append);
        tokio::select! {
            result = &mut append => panic!("proxy must hold the ACK: {result:?}"),
            _ = held.notified() => {},
            _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("broker did not commit"),
        }
        // Dropping the future bypasses all normal Result/error branches.
    }
    // It must refuse locally on the very first poll, before touching the still
    // connected transport. A later socket failure would hide the regression.
    assert!(matches!(
        futures::poll!(Box::pin(first.append(0, &command))),
        std::task::Poll::Ready(Err(LogError::Unavailable))
    ));
    proxy.abort();
    let mut next = fixture.connect(2).await.unwrap();
    let recovered = next.replay_from_start(1, 10, 100_000).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].command.command_id, command.command_id);
}
