//! Actual authenticated TCP connection_loop and HTTP router, backed by real
//! broker/storage nodes. Listener/connection tasks are owned by this fixture.
#[cfg(feature = "river-perf")]
#[path = "river_perf.rs"]
mod river_perf;
use super::*;
use crate::{CowchatServer, ServerConfig};
use cowchat_client::{ClientError, CowchatClient};
use cowchat_core::{ErrorCode, FrameType};

struct Network {
    server: Arc<CowchatServer>,
    tcp: tokio::task::JoinHandle<()>,
    http: tokio::task::JoinHandle<()>,
    address: String,
    http_address: String,
    _directory: tempfile::TempDir,
}
impl Drop for Network {
    fn drop(&mut self) {
        self.tcp.abort();
        self.http.abort();
        self.server.broker().agents.clear();
    }
}
impl Network {
    async fn new(runtime: OwnerRuntime) -> Self {
        let directory = private_directory();
        let server = Arc::new(
            CowchatServer::new_hosted(
                ServerConfig {
                    socket_path: directory.path().join("server.sock"),
                    tcp_addr: None,
                    http_addr: None,
                    db_path: directory.path().join("auth.sqlite"),
                    auth_key_path: directory.path().join("auth.key"),
                    no_auth: false,
                    allow_keyless_local: false,
                    allow_private_webhooks: false,
                    http_signup_enabled: false,
                    http_admin_secret: None,
                    http_allowed_origins: vec![],
                    trusted_proxy_ips: vec![],
                    blob_idle_expiry_seconds: crate::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
                },
                runtime,
            )
            .unwrap(),
        );
        let votes = Arc::new(crate::voting::VoteManager::new(
            server.store().clone(),
            server.broker().clone(),
        ));
        let webhooks = Arc::new(crate::webhooks::WebhookManager::new(
            server.store().clone(),
            false,
        ));
        let app = crate::web::router(crate::web::AppState {
            broker: server.broker().clone(),
            store: server.store().clone(),
            vote_mgr: votes.clone(),
            rate_limiter: server.rate_limiter().clone(),
            no_auth: false,
            api_key: server.api_key().into(),
            reconnect_mgr: server.reconnect_mgr().clone(),
            task_mgr: server.task_mgr().clone(),
            webhook_mgr: webhooks.clone(),
            signup_enabled: false,
            admin_secret: None,
            allowed_origins: vec![],
            trusted_proxy_ips: vec![],
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = listener.local_addr().unwrap().to_string();
        let http = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let tcp_server = server.clone();
        let tcp = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (socket,_)=accepted.unwrap();
                        let (read,write)=tokio::io::split(socket);
                        let server=tcp_server.clone();
                        let votes=votes.clone();
                        let webhooks=webhooks.clone();
                        connections.spawn(async move {
                            let _=crate::server::connection_loop(read,write,server.broker().clone(),server.store().clone(),votes,
                                server.api_key().into(),false,false,server.rate_limiter().clone(),server.reconnect_mgr().clone(),
                                server.task_mgr().clone(),webhooks,None).await;
                        });
                    }
                    _=connections.join_next(),if !connections.is_empty()=>{}
                }
            }
        });
        Self {
            server,
            tcp,
            http,
            address,
            http_address,
            _directory: directory,
        }
    }
    async fn client(&self, id: &str) -> CowchatClient {
        let mut client =
            CowchatClient::connect_tcp(&self.address, self.server.api_key(), id, Some(id), vec![])
                .await
                .unwrap();
        client.set_room_secret(b"test-room-secret-for-hosted-client-seam");
        client
    }
}

struct GateRegistry {
    inner: Arc<LocalManifestRegistry>,
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
    lose_reply: AtomicBool,
}
#[async_trait::async_trait]
impl ManifestRegistry for GateRegistry {
    async fn is_shard_live(&self, id: &ShardId, index: u8) -> Result<bool, HookError> {
        self.inner.is_shard_live(id, index).await
    }
    async fn commit_manifest_v2(
        &self,
        id: &VolumeId,
        prev: &ManifestRoot,
        root: &ManifestRoot,
        delta: i64,
        relays: Vec<RelayByteDelta>,
        added: Vec<TaggedShardRef>,
        removed: Vec<TaggedShardRef>,
        token: &[u8],
    ) -> Result<u64, HookError> {
        if self.hold.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
        let result = self
            .inner
            .commit_manifest_v2(id, prev, root, delta, relays, added, removed, token)
            .await?;
        if self.lose_reply.swap(false, Ordering::SeqCst) {
            return Err("injected lost archive reply".into());
        }
        Ok(result)
    }
}

async fn gated_archive(storage: &Storage, volume: Volume) -> (CbfsArchive, Arc<GateRegistry>) {
    drop(storage.initialize_archive(volume).await);
    let gate = Arc::new(GateRegistry {
        inner: storage.registry.clone(),
        hold: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
        lose_reply: AtomicBool::new(false),
    });
    (
        storage.archive(storage.reopen().await, gate.clone()).await,
        gate,
    )
}

async fn next_message(
    events: &mut tokio::sync::broadcast::Receiver<cowchat_client::Event>,
) -> cowchat_client::Event {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let event = events.recv().await.unwrap();
            if event.frame.frame_type == FrameType::MessageReceived {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn hosted_ingress_binds_key_epoch_and_never_uses_local_password_decryption() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let directory = private_directory();
    let mut runtime = recover(
        promote(&fixture, &mut writers).await,
        storage.initialize_archive(volume).await,
        &directory.path().join("intents.sqlite"),
    )
    .await;
    runtime
        .submit(vec![
            create("one", 0),
            key_prepare("one", 0, None),
            key_cutover("one", 0, None),
        ])
        .await
        .unwrap();
    let network = Network::new(runtime).await;
    let alice = network.client("alice").await;
    alice.join_room("one").await.unwrap();
    // prepare_message intentionally uses the local profile; labelling this
    // ciphertext tests that a keyed response is never implicitly decrypted by
    // that profile. Real senders use the separate contextual raw-key codec.
    let mut payload = alice.prepare_message("one", "private", None, vec![], serde_json::json!({}));
    for epoch in [
        None,
        Some("00"),
        Some("+0"),
        Some("18446744073709551616"),
        Some("1"),
    ] {
        payload.message_id = Some(uuid::Uuid::new_v4().to_string());
        payload.key_epoch = epoch.map(str::to_owned);
        assert!(alice.append_prepared_message(&payload).await.is_err());
    }
    payload.message_id = Some(uuid::Uuid::new_v4().to_string());
    payload.key_epoch = Some("0".into());
    let message = alice.append_prepared_message(&payload).await.unwrap();
    assert_eq!(message.key_epoch.as_deref(), Some("0"));
    assert_eq!(message.content, payload.content);
    let retry = alice.append_prepared_message(&payload).await.unwrap();
    assert_eq!(retry.message_id, message.message_id);
    let history = alice.get_history("one", 50, None).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, payload.content);
    assert_eq!(history[0].key_epoch.as_deref(), Some("0"));
    payload.key_epoch = Some("1".into());
    assert!(alice.append_prepared_message(&payload).await.is_err());
    let context = cowchat_core::room_crypto::Context {
        room_id: "one",
        key_epoch: 0,
        message_id: "raw-key-message",
    };
    let prepared = CowchatClient::prepare_room_key_message(
        &[42; 32],
        &context,
        "actual contextual message",
        None,
        vec![],
        serde_json::json!({}),
    )
    .unwrap();
    let received = alice.append_prepared_message(&prepared).await.unwrap();
    assert_eq!(received.content, prepared.content);
    assert_eq!(received.key_epoch.as_deref(), Some("0"));
    assert_eq!(
        cowchat_core::room_crypto::decrypt(&[42; 32], &context, &received.content).unwrap(),
        "actual contextual message"
    );
}

#[tokio::test]
async fn authenticated_clients_wait_for_archive_while_history_and_reconnect_read_committed_state() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let (archive, gate) = gated_archive(&storage, volume).await;
    let directory = private_directory();
    let runtime = recover(
        promote(&fixture, &mut writers).await,
        archive,
        &directory.path().join("intents.sqlite"),
    )
    .await;
    let network = Network::new(runtime).await;
    let alice = Arc::new(network.client("alice").await);
    let bob = network.client("bob").await;
    let prepared_room = CowchatClient::prepare_hosted_room("one");
    let room = alice.create_prepared_room(&prepared_room).await.unwrap();
    assert_eq!(
        alice
            .create_prepared_room(&prepared_room)
            .await
            .unwrap()
            .room_id,
        room.room_id
    );
    alice.join_room(&room.room_id).await.unwrap();
    bob.join_room(&room.room_id).await.unwrap();
    let mut events = bob.subscribe();
    let prepared = alice.prepare_message(
        &room.room_id,
        "durable hello",
        None,
        vec![],
        serde_json::json!({}),
    );
    gate.hold.store(true, Ordering::SeqCst);
    let sender = alice.clone();
    let payload = prepared.clone();
    let send = tokio::spawn(async move { sender.append_prepared_message(&payload).await });
    tokio::time::timeout(DEADLINE, gate.entered.notified())
        .await
        .unwrap();
    assert!(
        !send.is_finished(),
        "caller ACK cannot precede archive commit"
    );
    assert!(tokio::time::timeout(
        Duration::from_secs(2),
        bob.get_history(&room.room_id, 50, None)
    )
    .await
    .unwrap()
    .unwrap()
    .is_empty());
    assert!(
        events.try_recv().is_err(),
        "pending message must not fan out"
    );
    // Take over Bob's live stable identity while Alice owns the write queue.
    // Registration's synchronous membership filters must see committed rooms.
    let replacement = tokio::time::timeout(Duration::from_secs(2), network.client("bob"))
        .await
        .unwrap();
    assert!(network
        .server
        .broker()
        .is_agent_in_room("bob", &room.room_id));
    let mut replacement_events = replacement.subscribe();
    gate.release.add_permits(1);
    let receipt = send.await.unwrap().unwrap();
    assert_eq!(receipt.content, "durable hello");
    assert_eq!(
        next_message(&mut replacement_events).await.frame.payload["message_id"],
        receipt.message_id
    );
    assert_eq!(
        replacement
            .get_history(&room.room_id, 50, None)
            .await
            .unwrap()[0]
            .content,
        "durable hello"
    );
    assert_eq!(
        alice.append_prepared_message(&prepared).await.unwrap().seq,
        receipt.seq
    );
    assert_eq!(
        replacement
            .get_history(&room.room_id, 50, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            next_message(&mut replacement_events)
        )
        .await
        .is_err(),
        "an exact retry must not emit another message event"
    );
    let changed = alice.prepare_message(
        &room.room_id,
        "other bytes",
        None,
        vec![],
        serde_json::json!({}),
    );
    let mut conflict = changed;
    conflict.message_id = prepared.message_id.clone();
    assert!(matches!(
        alice.append_prepared_message(&conflict).await,
        Err(ClientError::Server {
            code: ErrorCode::MessageConflict,
            ..
        })
    ));
    assert!(network
        .server
        .store()
        .get_room(&room.room_id)
        .unwrap()
        .is_none());
    assert!(network
        .server
        .store()
        .get_history(&room.room_id, 50, None)
        .unwrap()
        .is_empty());
    drop(network);
    drop(alice);
    drop(bob);
    drop(replacement);
    // Fresh owner process: no projection and no local pending journal copied.
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let fresh_directory = private_directory();
    let fresh = Network::new(
        recover(
            promote(&fixture, &mut writers).await,
            archive,
            &fresh_directory.path().join("intents.sqlite"),
        )
        .await,
    )
    .await;
    let alice = fresh.client("alice").await;
    assert_eq!(
        alice
            .create_prepared_room(&prepared_room)
            .await
            .unwrap()
            .room_id,
        room.room_id
    );
    alice.join_room(&room.room_id).await.unwrap();
    assert_eq!(
        alice.get_history(&room.room_id, 50, None).await.unwrap()[0].message_id,
        receipt.message_id
    );
    assert_eq!(
        alice.append_prepared_message(&prepared).await.unwrap().seq,
        receipt.seq
    );
}

#[tokio::test]
async fn unknown_archive_result_retires_all_authenticated_reads_and_recovers_the_original_receipt()
{
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let (archive, gate) = gated_archive(&storage, volume).await;
    let directory = private_directory();
    let network = Network::new(
        recover(
            promote(&fixture, &mut writers).await,
            archive,
            &directory.path().join("intents.sqlite"),
        )
        .await,
    )
    .await;
    let alice = network.client("alice").await;
    let bob = network.client("bob").await;
    let room = alice
        .create_prepared_room(&CowchatClient::prepare_hosted_room("one"))
        .await
        .unwrap();
    alice.join_room(&room.room_id).await.unwrap();
    bob.join_room(&room.room_id).await.unwrap();
    let prepared = alice.prepare_message(
        &room.room_id,
        "unknown ACK",
        None,
        vec![],
        serde_json::json!({}),
    );
    gate.lose_reply.store(true, Ordering::SeqCst);
    assert!(alice.append_prepared_message(&prepared).await.is_err());
    assert!(bob.get_history(&room.room_id, 50, None).await.is_err());
    assert!(bob.join_room(&room.room_id).await.is_err());
    drop(network);
    drop(alice);
    drop(bob);
    let fresh_directory = private_directory();
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let fresh = Network::new(
        recover(
            promote(&fixture, &mut writers).await,
            archive,
            &fresh_directory.path().join("intents.sqlite"),
        )
        .await,
    )
    .await;
    let alice = fresh.client("alice").await;
    alice.join_room(&room.room_id).await.unwrap();
    let receipt = alice.append_prepared_message(&prepared).await.unwrap();
    assert_eq!(receipt.seq, 1);
    assert_eq!(
        alice
            .get_history(&room.room_id, 50, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn hosted_denies_unbound_credentials_unsupported_mutations_and_http_bypasses() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let directory = private_directory();
    let network = Network::new(
        recover(
            promote(&fixture, &mut writers).await,
            storage.initialize_archive(volume).await,
            &directory.path().join("intents.sqlite"),
        )
        .await,
    )
    .await;
    let alice = network.client("alice").await;
    let room = alice
        .create_prepared_room(&CowchatClient::prepare_hosted_room("one"))
        .await
        .unwrap();
    alice.join_room(&room.room_id).await.unwrap();
    assert!(matches!(
        alice.rename_room(&room.room_id, "changed").await,
        Err(ClientError::Server {
            code: ErrorCode::UnsupportedProtocol,
            ..
        })
    ));
    network
        .server
        .store()
        .create_api_key("other-valid-test-key", None)
        .unwrap();
    let outsider = CowchatClient::connect_tcp(
        &network.address,
        "other-valid-test-key",
        "outsider",
        Some("outsider"),
        vec![],
    )
    .await
    .unwrap();
    assert!(matches!(
        outsider.join_room(&room.room_id).await,
        Err(ClientError::Server {
            code: ErrorCode::AccessDenied,
            ..
        })
    ));
    assert!(outsider.get_history(&room.room_id, 50, None).await.is_err());
    let mut web_client = CowchatClient::connect_ws(
        &format!("ws://{}/ws", network.http_address),
        network.server.api_key(),
        "web-owner",
        Some("web-owner"),
        vec![],
    )
    .await
    .unwrap();
    web_client.set_room_secret(b"test-room-secret-for-hosted-client-seam");
    web_client.join_room(&room.room_id).await.unwrap();
    let prepared = web_client.prepare_message(
        &room.room_id,
        "WebSocket shares the owner log",
        None,
        vec![],
        serde_json::json!({}),
    );
    web_client.append_prepared_message(&prepared).await.unwrap();
    assert_eq!(
        alice.get_history(&room.room_id, 50, None).await.unwrap()[0].content,
        "WebSocket shares the owner log"
    );
    let http = reqwest::Client::new();
    for path in [
        "/api/rooms",
        "/api/invites/redeem",
        "/api/rooms/one/blobs",
        "/api/keys",
    ] {
        let response = http
            .post(format!("http://{}{path}", network.http_address))
            .header("x-cowchat-key", network.server.api_key())
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
    }
    assert!(network
        .server
        .store()
        .get_room(&room.room_id)
        .unwrap()
        .is_none());
}
