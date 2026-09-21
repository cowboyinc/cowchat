use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

fn update(id: i64, chat: i64, bot: bool, text: &str) -> Value {
    json!({"update_id":id,"message":{"chat":{"id":chat},"from":{"id":42,"is_bot":bot},"text":text}})
}

fn fixture_bridge(path: PathBuf, room: &str) -> RoomBridge {
    RoomBridge {
        room: room.into(),
        secret: b"secret".to_vec(),
        state_file: path,
        mentions: vec!["actor".into()],
        skip_room_seq: None,
    }
}

#[test]
fn inbound_allowlist_encrypts_sender_and_message_and_preserves_mentions() {
    let bridge = fixture_bridge(PathBuf::new(), "room");
    let accepted = bridge.prepare(
        inbound(
            &update(10, -99, false, "private text"),
            -99,
            "bridge",
            "123",
        )
        .unwrap()
        .unwrap(),
        Some(json!(11)),
    );
    assert_eq!(
        accepted.message.message_id.as_deref(),
        Some("telegram:bridge:123:10")
    );
    assert_eq!(accepted.message.mentions, vec!["actor"]);
    assert_eq!(
        cowchat_core::crypto::decrypt(b"secret", "room", &accepted.message.content).unwrap(),
        "Telegram user 42: private text"
    );
    assert!(!serde_json::to_string(&accepted)
        .unwrap()
        .contains("private text"));
    for rejected in [
        update(11, -100, false, "other chat"),
        update(12, -99, true, "bot echo"),
        json!({"update_id":13,"edited_message":{"text":"edited"}}),
    ] {
        assert!(inbound(&rejected, -99, "bridge", "123").unwrap().is_none());
    }
    assert!(inbound(
        &update(i64::MAX, -99, false, "overflow"),
        -99,
        "bridge",
        "123"
    )
    .is_err());
}

#[test]
fn chunks_preserve_unicode_without_exceeding_telegram_limit() {
    let text = "🐎<&>".repeat(3000);
    let split = chunks(&text);
    assert_eq!(split.concat(), text);
    assert!(split.iter().all(|part| part.encode_utf16().count() <= 4000));
}

#[test]
fn state_lock_excludes_second_bridge_and_atomic_save_contains_no_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.tmp");
    let lock_path = sidecar(&path, ".lock");
    let lock = private_file(&lock_path).unwrap();
    lock.try_lock().unwrap();
    let second = private_file(&lock_path).unwrap();
    assert!(second.try_lock().is_err());
    let state = State {
        binding: "mapping".into(),
        external_cursor: None,
        reported_block: None,
        room_seq: 0,
        pending: Some(
            fixture_bridge(path.clone(), "room").prepare(
                inbound(&update(1, 7, false, "secret text"), 7, "bridge", "123")
                    .unwrap()
                    .unwrap(),
                Some(json!(2)),
            ),
        ),
    };
    save(&path, &state).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("secret text"));
    let restored: State = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        restored.pending.unwrap().message.content,
        state.pending.unwrap().message.content
    );
    drop(lock);
    second.try_lock().unwrap();
}

#[tokio::test]
async fn encrypted_bridge_recovers_append_and_retries_telegram_without_advancing_cursor() {
    use cowchat_server::{CowchatServer, ServerConfig};
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    let server = CowchatServer::new(ServerConfig {
        socket_path: dir.path().join("server.sock"),
        tcp_addr: Some(addr.clone()),
        http_addr: None,
        db_path: dir.path().join("server.db"),
        auth_key_path: dir.path().join("auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: false,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
    })
    .unwrap();
    let key = server.api_key().to_owned();
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let bridge = CowchatClient::connect_tcp(&addr, &key, "Telegram", Some("bridge"), vec![])
        .await
        .unwrap();
    let mut human = CowchatClient::connect_tcp(&addr, &key, "Human", Some("human"), vec![])
        .await
        .unwrap();
    human.set_room_secret(b"secret");
    let room = human
        .create_room_with_options("bridge-test", None, None, false, true)
        .await
        .unwrap()
        .room_id;
    bridge.join_room(&room).await.unwrap();
    human.join_room(&room).await.unwrap();
    let mut config = fixture_bridge(dir.path().join("cursor.json"), &room);
    let mut state = State {
        binding: "test".into(),
        external_cursor: None,
        reported_block: None,
        room_seq: 0,
        pending: Some(
            config.prepare(
                inbound(&update(10, -99, false, "hello"), -99, "bridge", "123")
                    .unwrap()
                    .unwrap(),
                Some(json!(11)),
            ),
        ),
    };
    save(&config.state_file, &state).unwrap();
    bridge
        .append_prepared_message(&state.pending.as_ref().unwrap().message)
        .await
        .unwrap();
    // Simulated crash after append, before cursor save: reload the exact ciphertext.
    state = serde_json::from_slice(&std::fs::read(&config.state_file).unwrap()).unwrap();
    flush_pending(&bridge, &mut state, &config.state_file)
        .await
        .unwrap();
    assert_eq!(state.external_cursor, Some(json!(11)));
    assert_eq!(human.get_history(&room, 10, None).await.unwrap().len(), 1);
    assert!(bridge.get_history(&room, 10, None).await.unwrap()[0]
        .content
        .starts_with("cow1:"));

    let fail = Arc::new(AtomicBool::new(true));
    let sent = Arc::new(Mutex::new(Vec::<Value>::new()));
    let fail_handler = fail.clone();
    let sent_handler = sent.clone();
    let api = axum::Router::new()
        .route("/botfixture/getUpdates", axum::routing::post(|| async {
            axum::Json(json!({"ok":true,"result":[update(10,-99,false,"hello"),update(11,-100,false,"foreign"),update(12,-99,true,"echo")]}))
        }))
        .route("/botfixture/sendMessage", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let fail = fail_handler.clone(); let sent = sent_handler.clone(); async move {
                if fail.load(Ordering::SeqCst) {
                    axum::Json(json!({"ok":false,"error_code":429,"description":"BOT_TOKEN_SENTINEL","parameters":{"retry_after":1}}))
                } else {
                    sent.lock().unwrap().push(body);
                    axum::Json(json!({"ok":true,"result":{"message_id":55}}))
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let telegram = Telegram {
        http: reqwest::Client::new(),
        base: format!("http://{}/botfixture", listener.local_addr().unwrap()),
        chat_id: -99,
    };
    let http = tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
    human
        .send_message(&room, "private reply 🐎", None, vec![])
        .await
        .unwrap();
    let error = pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("BOT_TOKEN_SENTINEL"));
    assert_eq!(state.external_cursor, Some(json!(13))); // ignored foreign/bot updates are acknowledged
    assert_eq!(state.room_seq, 1); // failed outbound message is still pending
    fail.store(false, Ordering::SeqCst);
    state = serde_json::from_slice(&std::fs::read(&config.state_file).unwrap()).unwrap();
    pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .unwrap();
    assert_eq!(state.room_seq, 2);
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &[
            json!({"chat_id":-99,"text":"Human: private reply 🐎","link_preview_options":{"is_disabled":true}})
        ]
    );
    assert_eq!(human.get_history(&room, 10, None).await.unwrap().len(), 2);

    // Wrong-key/poison ciphertext must not silently advance the cursor.
    let mut poison = config
        .prepare(
            ExternalMessage {
                message_id: "poison".into(),
                sender: "Human".into(),
                text: "wrong-key text".into(),
            },
            None,
        )
        .message;
    poison.content = cowchat_core::crypto::encrypt(b"wrong", &room, "unreadable");
    human.append_prepared_message(&poison).await.unwrap();
    human
        .send_message(&room, "after poison", None, vec![])
        .await
        .unwrap();
    assert!(pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .is_err());
    assert_eq!(state.room_seq, 2);
    assert_eq!(sent.lock().unwrap()[1]["text"], "[Bridge paused at room sequence 3: message could not be decrypted. Operator action required.]");
    assert_eq!(state.reported_block, Some(3));
    state = serde_json::from_slice(&std::fs::read(&config.state_file).unwrap()).unwrap();
    config.skip_room_seq = Some(4); // cannot accidentally skip a range or a different seq
    assert!(pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .is_err());
    assert_eq!(state.room_seq, 2);
    assert_eq!(sent.lock().unwrap().len(), 2); // pause notice is not spammed on retries/restart
    config.skip_room_seq = Some(3);
    fail.store(true, Ordering::SeqCst);
    assert!(pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .is_err());
    assert_eq!(state.room_seq, 2); // skip notice must also be accepted first
    fail.store(false, Ordering::SeqCst);
    pump(&bridge, &telegram, &config, &mut state, "123")
        .await
        .unwrap();
    assert_eq!(state.room_seq, 4);
    assert_eq!(
        sent.lock().unwrap()[2]["text"],
        "[Bridge operator skipped unreadable room message at sequence 3]"
    );
    assert_eq!(sent.lock().unwrap()[3]["text"], "Human: after poison");

    // A predictable ID occupied by another participant is not success.
    state.pending = Some(
        config.prepare(
            inbound(
                &update(13, -99, false, "real Telegram text"),
                -99,
                "bridge",
                "123",
            )
            .unwrap()
            .unwrap(),
            Some(json!(14)),
        ),
    );
    let mut forged = state.pending.as_ref().unwrap().message.clone();
    forged.content = cowchat_core::crypto::encrypt(b"secret", &room, "different sender/content");
    human.append_prepared_message(&forged).await.unwrap();
    save(&config.state_file, &state).unwrap();
    assert!(flush_pending(&bridge, &mut state, &config.state_file)
        .await
        .is_err());
    assert!(state.pending.is_some());
    assert_eq!(state.external_cursor, Some(json!(13)));
    http.abort();
    task.abort();
}
