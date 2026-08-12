use async_trait::async_trait;
use cowchat_client::CowchatClient;
use cowchat_codex::app_server::{AppServerError, CodexWakeOutcome, WakeBackend, WakeReference};
use cowchat_codex::config::{
    BridgeConfig, BridgeRole, CodexConfig, CowchatConfig, RelayConfig, TargetConfig, WakeHint,
};
use cowchat_codex::relay::WakeRelay;
use cowchat_codex::service::{
    ChatBackend, CowchatBackend, ServiceError, WakeInboxAckInput, WakeInboxReadInput, WakeService,
};
use cowchat_codex::store::WakeStore;
use cowchat_server::{CowchatServer, ServerConfig};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct RecordingWake {
    calls: Mutex<Vec<WakeReference>>,
}

#[async_trait]
impl WakeBackend for RecordingWake {
    async fn wake(
        &self,
        _thread_id: &str,
        reference: &WakeReference,
    ) -> Result<CodexWakeOutcome, AppServerError> {
        self.calls.lock().unwrap().push(reference.clone());
        Ok(CodexWakeOutcome {
            mode: "started".into(),
            prior_status: "idle".into(),
            turn_id: "turn-relay".into(),
        })
    }
}

#[tokio::test]
async fn ordinary_peer_message_is_durable_and_wakes_ended_target() {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let key_path = temp.path().join("auth.key");
    let server = CowchatServer::new(ServerConfig {
        socket_path: temp.path().join("cowchat.sock"),
        tcp_addr: Some(address.clone()),
        http_addr: None,
        db_path: temp.path().join("cowchat.db"),
        auth_key_path: key_path.clone(),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
    })
    .unwrap();
    let key = server.api_key().to_string();
    let server_task = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut owner = CowchatClient::connect_tcp(&address, &key, "owner", Some("room-owner"), vec![])
        .await
        .unwrap();
    let room = owner
        .create_room("relay-integration", None, None, false)
        .await
        .unwrap();

    let config = BridgeConfig {
        state_db: temp.path().join("wake.db"),
        cowchat: CowchatConfig {
            tcp: Some(address.clone()),
            socket: None,
            api_key_file: key_path.clone(),
            agent_name: "wake bridge".into(),
            agent_id: "wake-bridge-integration".into(),
            room_key_env: None,
        },
        codex: CodexConfig::default(),
        relay: RelayConfig {
            poll_interval_ms: 10,
        },
        targets: BTreeMap::from([(
            "reviewer".into(),
            TargetConfig {
                thread_id: "thread-reviewer".into(),
                room: room.room_id.clone(),
                agent_id: Some("recipient-agent".into()),
                relay: true,
                min_wake_hint: WakeHint::Normal,
            },
        )]),
    };
    let store = Arc::new(WakeStore::open(&config.state_db, &config.state_scope()).unwrap());
    let backend = Arc::new(CowchatBackend::for_role(
        config.cowchat.clone(),
        BridgeRole::Relay,
    ));
    let relay_agent_id = config.cowchat.role_agent_id(BridgeRole::Relay);
    let target_identity = config.target_identity("reviewer").unwrap();
    let wake = Arc::new(RecordingWake::default());
    let service = WakeService::new(config.clone(), store.clone(), backend.clone(), wake.clone());
    let relay = WakeRelay::new(config, store.clone(), backend, service.clone());
    assert_eq!(relay.run_once(false).await.unwrap(), 0);
    let handle = store
        .current_target(&target_identity, "reviewer", &room.room_id)
        .unwrap()
        .unwrap();
    let initial_source_cursor = store.relay_cursor(&handle).unwrap().unwrap();

    let peer = CowchatClient::connect_tcp(&address, &key, "peer", Some("peer-agent"), vec![])
        .await
        .unwrap();
    peer.join_room(&room.room_id).await.unwrap();
    let message = peer
        .send_message(&room.room_id, "natural follow-up", None, vec![])
        .await
        .unwrap();

    assert_eq!(relay.run_once(false).await.unwrap(), 1);
    assert_eq!(wake.calls.lock().unwrap().len(), 1);
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
    assert_eq!(inbox.events[0].event.event_type, "cowchat.message.received");
    assert_eq!(inbox.events[0].event.data["message_id"], message.message_id);
    assert_eq!(inbox.events[0].event.data["seq"], message.seq);
    let room_tip_after_wake = peer.room_tip(&room.room_id).await.unwrap();
    assert_eq!(
        store.relay_cursor(&handle).unwrap(),
        Some(initial_source_cursor),
        "the source cursor remains pinned until the wake is acknowledged"
    );
    service
        .acknowledge(WakeInboxAckInput {
            target: "reviewer".into(),
            state_id: inbox.state_id,
            cursor: inbox.highest_returned_seq,
        })
        .await
        .unwrap();
    assert_eq!(relay.run_once(false).await.unwrap(), 0);
    assert_eq!(
        store.relay_cursor(&handle).unwrap(),
        Some(room_tip_after_wake)
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let agents = owner.list_agents(Some(&room.room_id)).await.unwrap();
    assert!(
        agents.iter().any(|agent| agent.agent_id == relay_agent_id),
        "relay keeps its one role-scoped Cowchat connection alive across scans"
    );

    owner.set_room_secret(b"correct-room-secret");
    let encrypted_room = owner
        .create_room_with_options("relay-encrypted", None, None, false, false, true)
        .await
        .unwrap();
    owner.join_room(&encrypted_room.room_id).await.unwrap();
    owner
        .send_message(&encrypted_room.room_id, "ciphertext probe", None, vec![])
        .await
        .unwrap();
    let room_key_env = format!(
        "COWCHAT_CODEX_TEST_ROOM_KEY_{}",
        uuid::Uuid::new_v4().simple()
    );
    std::env::set_var(&room_key_env, "wrong-room-secret");
    let mut encrypted_config = config_for_room_key(&address, &key_path, &room_key_env);
    let wrong_key = CowchatBackend::for_role(encrypted_config.clone(), BridgeRole::Doctor);
    assert!(matches!(
        wrong_key.inspect_room(&encrypted_room.room_id).await,
        Err(ServiceError::InvalidRoomKey(_))
    ));
    std::env::set_var(&room_key_env, "correct-room-secret");
    encrypted_config.agent_id.push_str("-correct");
    let correct_key = CowchatBackend::for_role(encrypted_config, BridgeRole::Doctor);
    assert_eq!(
        correct_key
            .inspect_room(&encrypted_room.room_id)
            .await
            .unwrap()
            .key_validation,
        "verified_from_history"
    );
    std::env::remove_var(room_key_env);

    server_task.abort();
}

fn config_for_room_key(address: &str, key_path: &std::path::Path, env_name: &str) -> CowchatConfig {
    CowchatConfig {
        tcp: Some(address.to_string()),
        socket: None,
        api_key_file: key_path.to_path_buf(),
        agent_name: "wake bridge key probe".into(),
        agent_id: "wake-bridge-key-probe-01989f2d".into(),
        room_key_env: Some(env_name.to_string()),
    }
}
