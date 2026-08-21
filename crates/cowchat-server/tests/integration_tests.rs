use cowchat_client::CowchatClient;
use cowchat_core::{Frame, FrameType, RegisterPayload, VoteStatus};
use cowchat_server::{CowchatServer, ServerConfig};
use std::time::Duration;
use tokio::time::sleep;

/// Start a test server on a random TCP port.
async fn start_test_server() -> (
    tokio::task::JoinHandle<()>,
    String,
    String,
    tempfile::TempDir,
) {
    start_test_server_with_keyless_local(false).await
}

async fn start_test_server_with_keyless_local(
    allow_keyless_local: bool,
) -> (
    tokio::task::JoinHandle<()>,
    String,
    String,
    tempfile::TempDir,
) {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let key_path = tmp_dir.path().join("auth.key");
    let socket_path = tmp_dir.path().join("test.sock");

    // Find a free port
    let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp_listener.local_addr().unwrap();
    let tcp_addr = addr.to_string();
    drop(tcp_listener);

    let config = ServerConfig {
        socket_path,
        tcp_addr: Some(tcp_addr.clone()),
        http_addr: None,
        db_path,
        auth_key_path: key_path,
        no_auth: false,
        allow_keyless_local,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
    };

    let server = CowchatServer::new(config).unwrap();
    let api_key = server.api_key().to_string();

    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    sleep(Duration::from_millis(100)).await;

    (handle, tcp_addr, api_key, tmp_dir)
}

async fn connect_agent(addr: &str, key: &str, name: &str) -> CowchatClient {
    CowchatClient::connect_tcp(addr, key, name, None, vec![])
        .await
        .unwrap()
}

async fn receives_room_event(
    events: &mut tokio::sync::broadcast::Receiver<cowchat_client::Event>,
    frame_type: FrameType,
    room_id: &str,
    timeout: Duration,
) -> bool {
    tokio::time::timeout(timeout, async {
        loop {
            if let Ok(event) = events.recv().await {
                if event.frame.frame_type == frame_type
                    && event
                        .frame
                        .payload
                        .get("room_id")
                        .and_then(|value| value.as_str())
                        == Some(room_id)
                {
                    return true;
                }
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Connect an agent tagged with a model capability (read from COWCHAT_MODEL env, default claude-opus-4.6).
async fn connect_agent_with_model(addr: &str, key: &str, name: &str) -> CowchatClient {
    let model = std::env::var("COWCHAT_MODEL").unwrap_or_else(|_| "claude-opus-4.6".to_string());
    CowchatClient::connect_tcp(addr, key, name, None, vec![format!("model:{}", model)])
        .await
        .unwrap()
}

/// Register a raw TCP client, then send invalid UTF-8 to trigger a server read error.
async fn register_then_trigger_read_error(addr: &str, key: &str, name: &str) {
    let addr = addr.to_string();
    let key = key.to_string();
    let name = name.to_string();

    tokio::task::spawn_blocking(move || {
        use std::io::{BufRead, BufReader, Write};

        let mut stream = std::net::TcpStream::connect(&addr).unwrap();
        stream.set_nodelay(true).unwrap();

        let register = Frame {
            id: Some("req-raw-register".to_string()),
            reply_to: None,
            frame_type: FrameType::Register,
            payload: serde_json::to_value(RegisterPayload {
                key,
                agent_id: None,
                name,
                capabilities: vec![],
                reconnect: false,
                protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
            })
            .unwrap(),
        };

        stream
            .write_all(register.to_line().unwrap().as_bytes())
            .unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response = Frame::from_line(&line).unwrap();
        assert_eq!(response.frame_type, FrameType::Ok);
        drop(reader);

        // Trigger read_line(String) failure on the server.
        stream.write_all(&[0xFF, 0xFE, b'\n']).unwrap();
        drop(stream);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_register_and_ping() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let client = connect_agent(&addr, &key, "test-agent").await;
    client.ping().await.unwrap();
}

#[tokio::test]
async fn test_invalid_key_rejected() {
    let (_handle, addr, _key, _tmp) = start_test_server().await;
    let result = CowchatClient::connect_tcp(&addr, "wrong-key", "bad-agent", None, vec![]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_loopback_client_can_connect_without_key_when_enabled() {
    let (_handle, addr, _key, _tmp) = start_test_server_with_keyless_local(true).await;
    let client = CowchatClient::connect_tcp(&addr, "", "local-agent", None, vec![])
        .await
        .expect("a trusted loopback client should not need an API key");
    client.ping().await.unwrap();
}

#[tokio::test]
async fn test_keyless_local_rejects_invalid_nonempty_key() {
    let (_handle, addr, _key, _tmp) = start_test_server_with_keyless_local(true).await;
    let result = CowchatClient::connect_tcp(
        &addr,
        "invalid-but-nonempty",
        "invalid-local-agent",
        None,
        vec![],
    )
    .await;
    assert!(
        result.is_err(),
        "local trust permits an empty key, not an arbitrary credential"
    );
}

#[tokio::test]
async fn test_keyless_local_and_authenticated_private_rooms_are_isolated() {
    let (_handle, addr, key, _tmp) = start_test_server_with_keyless_local(true).await;
    let keyless_owner =
        CowchatClient::connect_tcp(&addr, "", "keyless-owner", Some("keyless-owner"), vec![])
            .await
            .unwrap();
    let keyless_observer = CowchatClient::connect_tcp(
        &addr,
        "",
        "keyless-observer",
        Some("keyless-observer"),
        vec![],
    )
    .await
    .unwrap();
    let authenticated =
        CowchatClient::connect_tcp(&addr, &key, "authenticated", Some("authenticated"), vec![])
            .await
            .unwrap();
    let mut keyless_events = keyless_observer.subscribe();
    let mut authenticated_events = authenticated.subscribe();

    let keyless_room = keyless_owner
        .create_room("keyless-private", None, None)
        .await
        .unwrap();
    assert!(
        receives_room_event(
            &mut keyless_events,
            FrameType::RoomCreated,
            &keyless_room.room_id,
            Duration::from_secs(2),
        )
        .await
    );
    assert!(
        !receives_room_event(
            &mut authenticated_events,
            FrameType::RoomCreated,
            &keyless_room.room_id,
            Duration::from_millis(150),
        )
        .await
    );
    assert!(keyless_observer
        .room_info(&keyless_room.room_id)
        .await
        .is_ok());
    keyless_observer
        .join_room(&keyless_room.room_id)
        .await
        .unwrap();
    assert!(authenticated
        .room_info(&keyless_room.room_id)
        .await
        .is_err());
    assert!(authenticated
        .join_room(&keyless_room.room_id)
        .await
        .is_err());
    assert!(authenticated
        .destroy_room(&keyless_room.room_id)
        .await
        .is_err());
    assert!(!authenticated
        .list_rooms(None)
        .await
        .unwrap()
        .iter()
        .any(|room| room.room_id == keyless_room.room_id));

    keyless_owner
        .rename_room(&keyless_room.room_id, "keyless-renamed")
        .await
        .unwrap();
    assert!(
        receives_room_event(
            &mut keyless_events,
            FrameType::RoomUpdated,
            &keyless_room.room_id,
            Duration::from_secs(2),
        )
        .await
    );
    keyless_owner
        .destroy_room(&keyless_room.room_id)
        .await
        .unwrap();
    assert!(
        receives_room_event(
            &mut keyless_events,
            FrameType::RoomDestroyed,
            &keyless_room.room_id,
            Duration::from_secs(2),
        )
        .await
    );
    assert!(
        !receives_room_event(
            &mut authenticated_events,
            FrameType::RoomDestroyed,
            &keyless_room.room_id,
            Duration::from_millis(150),
        )
        .await
    );

    let authenticated_room = authenticated
        .create_room("authenticated-private", None, None)
        .await
        .unwrap();
    assert!(
        !receives_room_event(
            &mut keyless_events,
            FrameType::RoomCreated,
            &authenticated_room.room_id,
            Duration::from_millis(150),
        )
        .await
    );
    assert!(keyless_observer
        .room_info(&authenticated_room.room_id)
        .await
        .is_err());
    assert!(keyless_observer
        .join_room(&authenticated_room.room_id)
        .await
        .is_err());
    assert!(keyless_observer
        .destroy_room(&authenticated_room.room_id)
        .await
        .is_err());
    assert!(!keyless_observer
        .list_rooms(None)
        .await
        .unwrap()
        .iter()
        .any(|room| room.room_id == authenticated_room.room_id));

    let keyless_second = keyless_owner
        .create_room("keyless-second", None, None)
        .await
        .unwrap();
    keyless_owner
        .join_room(&keyless_second.room_id)
        .await
        .unwrap();
    keyless_owner
        .leave_room(&keyless_second.room_id)
        .await
        .unwrap();
    // Leaving never destroys a room; it stays, still invisible to other keys.
    assert!(keyless_owner
        .room_info(&keyless_second.room_id)
        .await
        .is_ok());
    assert!(authenticated
        .room_info(&keyless_second.room_id)
        .await
        .is_err());
}

#[tokio::test]
async fn test_oversized_preauth_frame_is_disconnected_without_harming_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (_handle, addr, key, _tmp) = start_test_server().await;
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    stream
        .write_all(&vec![b'x'; 1024 * 1024 + 1])
        .await
        .unwrap();
    let mut byte = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
        .await
        .expect("server should reject an oversized pre-auth frame promptly")
        .unwrap();
    assert_eq!(closed, 0, "oversized connection should be closed");

    let healthy = connect_agent(&addr, &key, "healthy-after-oversize").await;
    healthy.ping().await.unwrap();
}

#[tokio::test]
async fn test_read_error_disconnect_removes_agent() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let observer = connect_agent(&addr, &key, "observer").await;
    register_then_trigger_read_error(&addr, &key, "bad-wire-agent").await;

    let mut found_stale = true;
    for _ in 0..20 {
        let agents = observer.list_agents(None).await.unwrap();
        found_stale = agents.iter().any(|a| a.name == "bad-wire-agent");
        if !found_stale {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    assert!(
        !found_stale,
        "abruptly disconnected agent should be removed"
    );
}

#[tokio::test]
async fn test_session_end_recorded_on_disconnect() {
    let (_handle, addr, key, tmp) = start_test_server().await;

    register_then_trigger_read_error(&addr, &key, "session-agent").await;

    let db_path = tmp.path().join("test.db");
    let mut open_sessions = i64::MAX;
    for _ in 0..20 {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        open_sessions = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_sessions WHERE disconnected_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if open_sessions == 0 {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        open_sessions, 0,
        "all sessions should be marked disconnected"
    );
}

#[tokio::test]
async fn test_list_rooms_includes_lobby() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let client = connect_agent(&addr, &key, "agent-a").await;
    let rooms = client.list_rooms(None).await.unwrap();
    assert!(rooms.iter().any(|r| r.name == "lobby"));
}

#[tokio::test]
async fn test_create_and_join_room() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let client = connect_agent(&addr, &key, "agent-a").await;

    let room = client
        .create_room("test-room", Some("A test room"), None)
        .await
        .unwrap();
    assert_eq!(room.name, "test-room");

    client.join_room(&room.room_id).await.unwrap();

    let rooms = client.list_rooms(None).await.unwrap();
    assert!(rooms.iter().any(|r| r.name == "test-room"));
}

#[tokio::test]
async fn test_two_agents_communicate() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_b = agent_b.subscribe();

    let msg = agent_a
        .send_message("lobby", "Hello from A!", None, vec![])
        .await
        .unwrap();
    assert_eq!(msg.content, "Hello from A!");
    assert_eq!(msg.agent_name, "agent-a");

    // The server sends a heartbeat Ping immediately on connect, which races
    // with the broadcast; loop until we see the actual chat message.
    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let e = events_b.recv().await.unwrap();
            if e.frame.frame_type == FrameType::MessageReceived {
                return e;
            }
        }
    })
    .await
    .expect("MessageReceived within 2s");
    assert_eq!(
        event
            .frame
            .payload
            .get("content")
            .unwrap()
            .as_str()
            .unwrap(),
        "Hello from A!"
    );
}

#[tokio::test]
async fn test_message_history() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let client = connect_agent(&addr, &key, "agent-a").await;
    client.join_room("lobby").await.unwrap();

    client
        .send_message("lobby", "msg 1", None, vec![])
        .await
        .unwrap();
    client
        .send_message("lobby", "msg 2", None, vec![])
        .await
        .unwrap();
    client
        .send_message("lobby", "msg 3", None, vec![])
        .await
        .unwrap();

    let history = client.get_history("lobby", 50, None).await.unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].content, "msg 1");
    assert_eq!(history[2].content, "msg 3");
}

#[tokio::test]
async fn test_seq_monotonic_per_room() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let client = connect_agent(&addr, &key, "agent-a").await;
    client.join_room("lobby").await.unwrap();

    // Create a second room to confirm seqs are independent.
    let other = client.create_room("other-room", None, None).await.unwrap();
    client.join_room(&other.room_id).await.unwrap();

    let m1 = client
        .send_message("lobby", "a", None, vec![])
        .await
        .unwrap();
    let m2 = client
        .send_message("lobby", "b", None, vec![])
        .await
        .unwrap();
    let n1 = client
        .send_message(&other.room_id, "x", None, vec![])
        .await
        .unwrap();
    let m3 = client
        .send_message("lobby", "c", None, vec![])
        .await
        .unwrap();

    assert_eq!((m1.seq, m2.seq, m3.seq), (1, 2, 3));
    assert_eq!(n1.seq, 1);

    assert_eq!(client.room_tip("lobby").await.unwrap(), 3);
    assert_eq!(client.room_tip(&other.room_id).await.unwrap(), 1);

    let new_room = client.create_room("empty-room", None, None).await.unwrap();
    assert_eq!(client.room_tip(&new_room.room_id).await.unwrap(), 0);
}

#[tokio::test]
async fn test_history_since_seq_filter() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let client = connect_agent(&addr, &key, "agent-a").await;
    client.join_room("lobby").await.unwrap();

    for i in 0..5 {
        client
            .send_message("lobby", &format!("msg {i}"), None, vec![])
            .await
            .unwrap();
    }

    let after_seq_2 = client
        .get_history_filtered("lobby", 50, None, None, Some(2))
        .await
        .unwrap();
    assert_eq!(after_seq_2.len(), 3);
    assert_eq!(after_seq_2[0].seq, 3);
    assert_eq!(after_seq_2[0].content, "msg 2");
    assert_eq!(after_seq_2[2].seq, 5);
}

#[tokio::test]
async fn test_join_does_not_emit_chat_message() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    let pre_count = agent_a.get_history("lobby", 50, None).await.unwrap().len();

    let mut events_a = agent_a.subscribe();
    agent_b.join_room("lobby").await.unwrap();

    // agent-a sees the AgentJoined event (live signal of presence) but NOT a
    // persisted MessageReceived. Joining is a presence change, not a chat post.
    let mut saw_agent_joined = false;
    let mut received_message = None;
    let collect = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            let Ok(evt) = events_a.recv().await else {
                break;
            };
            match evt.frame.frame_type {
                cowchat_core::FrameType::AgentJoined => saw_agent_joined = true,
                cowchat_core::FrameType::MessageReceived => {
                    received_message = Some(evt);
                    break;
                }
                _ => {}
            }
        }
    });
    let _ = collect.await;
    assert!(saw_agent_joined, "agent-a should see AgentJoined event");
    assert!(
        received_message.is_none(),
        "join must not generate a MessageReceived broadcast"
    );

    // History is unchanged — no system row added.
    let history = agent_a.get_history("lobby", 50, None).await.unwrap();
    assert_eq!(
        history.len(),
        pre_count,
        "join must not persist a system message"
    );
}

#[tokio::test]
async fn test_join_silent_no_history_change() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;
    agent_a.join_room("lobby").await.unwrap();

    let before = agent_a.get_history("lobby", 50, None).await.unwrap().len();
    agent_b.join_room("lobby").await.unwrap(); // silent
    let after = agent_a.get_history("lobby", 50, None).await.unwrap().len();
    assert_eq!(before, after, "silent join must not write to history");
}

#[tokio::test]
async fn test_room_persists_after_all_members_leave() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    let room = agent_a
        .create_room("temp-collab", None, None)
        .await
        .unwrap();
    let room_id = room.room_id.clone();

    agent_a.join_room(&room_id).await.unwrap();
    agent_b.join_room(&room_id).await.unwrap();

    agent_a
        .send_message(&room_id, "still here later", None, vec![])
        .await
        .unwrap();

    agent_a.leave_room(&room_id).await.unwrap();
    agent_b.leave_room(&room_id).await.unwrap();

    // Rooms are only removed by explicit destroy; leaving never destroys.
    sleep(Duration::from_millis(50)).await;
    let rooms = agent_a.list_rooms(None).await.unwrap();
    assert!(rooms.iter().any(|r| r.room_id == room_id));
    let history = agent_a.get_history(&room_id, 50, None).await.unwrap();
    assert_eq!(history.len(), 1);

    agent_a.destroy_room(&room_id).await.unwrap();
    let rooms = agent_a.list_rooms(None).await.unwrap();
    assert!(!rooms.iter().any(|r| r.room_id == room_id));
}

#[tokio::test]
async fn test_room_hierarchy() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let client = connect_agent(&addr, &key, "agent-a").await;

    let parent = client
        .create_room("project-alpha", Some("Main project"), None)
        .await
        .unwrap();

    let child = client
        .create_room("alpha-testing", Some("Testing"), Some(&parent.room_id))
        .await
        .unwrap();
    assert_eq!(child.parent_id, Some(parent.room_id.clone()));

    let info = client.room_info(&parent.room_id).await.unwrap();
    let sub_rooms = info.get("sub_rooms").unwrap().as_array().unwrap();
    assert_eq!(sub_rooms.len(), 1);
    assert_eq!(
        sub_rooms[0].get("name").unwrap().as_str().unwrap(),
        "alpha-testing"
    );
}

#[tokio::test]
async fn test_agent_list() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let _agent_b = connect_agent(&addr, &key, "agent-b").await;

    let agents = agent_a.list_agents(None).await.unwrap();
    assert_eq!(agents.len(), 2);

    let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"agent-a"));
    assert!(names.contains(&"agent-b"));
}

#[tokio::test]
async fn test_global_agent_list_is_scoped_to_api_key() {
    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "agent-list-other-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'other')",
            [key_b],
        )
        .unwrap();
    let agent_a = connect_agent(&addr, &key_a, "agent-a").await;
    let agent_b = CowchatClient::connect_tcp(&addr, key_b, "agent-b", None, vec![])
        .await
        .unwrap();
    let visible_a = agent_a.list_agents(None).await.unwrap();
    let visible_b = agent_b.list_agents(None).await.unwrap();
    assert_eq!(
        visible_a
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-a"]
    );
    assert_eq!(
        visible_b
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-b"]
    );
}

#[tokio::test]
async fn test_mention_is_constrained_to_room_members() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let other_key = "mention-outsider-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'mention outsider')",
            [other_key],
        )
        .unwrap();

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = CowchatClient::connect_tcp(&addr, other_key, "agent-b", None, vec![])
        .await
        .unwrap();

    agent_a.join_room("lobby").await.unwrap();

    let room = agent_b.create_room("other-room", None, None).await.unwrap();
    agent_b.join_room(&room.room_id).await.unwrap();

    let mut events_b = agent_b.subscribe();

    agent_a
        .send_message(
            "lobby",
            "Hey @agent-b check this",
            None,
            vec![agent_b.agent_id.clone()],
        )
        .await
        .unwrap();

    let leaked = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let e = events_b.recv().await.unwrap();
            if e.frame.frame_type == FrameType::Mention {
                return true;
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(!leaked, "a mention must not leak content outside the room");

    agent_b.join_room("lobby").await.unwrap();
    agent_a
        .send_message(
            "lobby",
            "member mention",
            None,
            vec![agent_b.agent_id.clone()],
        )
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events_b.recv().await.unwrap();
            if event.frame.frame_type == FrameType::Mention {
                return event;
            }
        }
    })
    .await
    .expect("a room member should receive its mention");
    assert_eq!(event.frame.frame_type, FrameType::Mention);
}

// --- Voting tests ---

#[tokio::test]
async fn test_sealed_vote_two_agents() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();
    let mut events_b = agent_b.subscribe();

    // Agent A creates a vote
    let vote_info = agent_a
        .create_vote(
            "lobby",
            "Which approach?",
            Some("Pick implementation strategy"),
            vec!["Approach A".to_string(), "Approach B".to_string()],
            None,
        )
        .await
        .unwrap();
    assert_eq!(vote_info.title, "Which approach?");
    assert_eq!(vote_info.options.len(), 2);
    assert_eq!(vote_info.eligible_voters, 2);

    // Both agents receive VoteCreated
    let event = tokio::time::timeout(Duration::from_secs(2), events_b.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.frame.frame_type, FrameType::VoteCreated);

    // Agent A votes
    let resp_a = agent_a.cast_vote(&vote_info.vote_id, 0).await.unwrap();
    assert_eq!(resp_a.get("votes_cast").unwrap().as_u64().unwrap(), 1);

    // Check status -- should show 1 vote but no reveal
    let status = agent_a.get_vote_status(&vote_info.vote_id).await.unwrap();
    assert_eq!(status.votes_cast, 1);

    // Agent B votes -- this should trigger the result
    let _resp_b = agent_b.cast_vote(&vote_info.vote_id, 1).await.unwrap();

    // Drain events until we get VoteResult
    let result_event = loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events_a.recv())
            .await
            .unwrap()
            .unwrap();
        if event.frame.frame_type == FrameType::VoteResult {
            break event;
        }
    };

    // Verify results are revealed
    let tally = result_event
        .frame
        .payload
        .get("tally")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(tally.len(), 2);
    // Option 0 got 1 vote, option 1 got 1 vote
    let count_0 = tally[0].get("count").unwrap().as_u64().unwrap();
    let count_1 = tally[1].get("count").unwrap().as_u64().unwrap();
    assert_eq!(count_0, 1);
    assert_eq!(count_1, 1);

    // Ballots should be revealed
    let ballots = result_event
        .frame
        .payload
        .get("ballots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(ballots.len(), 2);
}

#[tokio::test]
async fn test_get_vote_status_after_close_returns_tally() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();

    let vote_info = agent_a
        .create_vote(
            "lobby",
            "Status after close",
            None,
            vec!["A".to_string(), "B".to_string()],
            None,
        )
        .await
        .unwrap();

    agent_a.cast_vote(&vote_info.vote_id, 0).await.unwrap();
    agent_b.cast_vote(&vote_info.vote_id, 1).await.unwrap();

    // Wait for vote closure event.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::VoteResult {
                    break;
                }
            }
        }
    })
    .await
    .expect("VoteResult should arrive");

    let status = agent_a.get_vote_status(&vote_info.vote_id).await.unwrap();
    assert_eq!(status.status, VoteStatus::Closed);
    assert_eq!(status.votes_cast, 2);
    assert_eq!(status.eligible_voters, 2);

    let tally = status
        .tally
        .expect("closed vote status should include tally");
    assert_eq!(tally.len(), 2);
    let count_a = tally
        .iter()
        .find(|row| row.option_index == 0)
        .map(|row| row.count)
        .unwrap_or(0);
    let count_b = tally
        .iter()
        .find(|row| row.option_index == 1)
        .map(|row| row.count)
        .unwrap_or(0);
    assert_eq!(count_a, 1);
    assert_eq!(count_b, 1);
}

#[tokio::test]
async fn test_open_vote_and_ballots_survive_server_restart() {
    let (handle, addr, key, tmp) = start_test_server().await;
    let first = CowchatClient::connect_tcp(&addr, &key, "first", Some("vote-first"), vec![])
        .await
        .unwrap();
    let second = CowchatClient::connect_tcp(&addr, &key, "second", Some("vote-second"), vec![])
        .await
        .unwrap();
    first.join_room("lobby").await.unwrap();
    second.join_room("lobby").await.unwrap();
    let vote = first
        .create_vote(
            "lobby",
            "survive restart",
            None,
            vec!["yes".into(), "no".into()],
            None,
        )
        .await
        .unwrap();
    first.cast_vote(&vote.vote_id, 0).await.unwrap();
    drop(first);
    drop(second);
    handle.abort();
    let _ = handle.await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let restarted_addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    let restarted = CowchatServer::new(ServerConfig {
        socket_path: tmp.path().join("vote-restart.sock"),
        tcp_addr: Some(restarted_addr.clone()),
        http_addr: None,
        db_path: tmp.path().join("test.db"),
        auth_key_path: tmp.path().join("auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
    })
    .unwrap();
    let restarted_handle = tokio::spawn(async move {
        let _ = restarted.run().await;
    });
    sleep(Duration::from_millis(100)).await;
    let first = CowchatClient::connect_tcp(
        &restarted_addr,
        &key,
        "first-after",
        Some("vote-first"),
        vec![],
    )
    .await
    .unwrap();
    let second = CowchatClient::connect_tcp(
        &restarted_addr,
        &key,
        "second-after",
        Some("vote-second"),
        vec![],
    )
    .await
    .unwrap();
    let replacement = CowchatClient::connect_tcp(
        &restarted_addr,
        &key,
        "replacement",
        Some("vote-replacement"),
        vec![],
    )
    .await
    .unwrap();
    first.join_room("lobby").await.unwrap();
    second.join_room("lobby").await.unwrap();
    replacement.join_room("lobby").await.unwrap();
    assert!(
        replacement.cast_vote(&vote.vote_id, 0).await.is_err(),
        "a replacement room member must not enter the frozen electorate"
    );
    second.cast_vote(&vote.vote_id, 1).await.unwrap();
    let status = first.get_vote_status(&vote.vote_id).await.unwrap();
    assert_eq!(status.status, VoteStatus::Closed);
    assert_eq!(status.votes_cast, 2);
    restarted_handle.abort();
}

#[tokio::test]
async fn test_list_votes_includes_closed_vote_history() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();

    let vote_info = agent_a
        .create_vote(
            "lobby",
            "History vote",
            None,
            vec!["Yes".to_string(), "No".to_string()],
            None,
        )
        .await
        .unwrap();

    agent_a.cast_vote(&vote_info.vote_id, 0).await.unwrap();
    agent_b.cast_vote(&vote_info.vote_id, 0).await.unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::VoteResult {
                    break;
                }
            }
        }
    })
    .await
    .expect("VoteResult should arrive");

    let votes = agent_a.list_votes("lobby", 10).await.unwrap();
    let found = votes
        .iter()
        .find(|v| v.vote_id == vote_info.vote_id)
        .expect("closed vote should appear in history");

    assert_eq!(found.status, VoteStatus::Closed);
    assert!(found.tally.is_some());
}

#[tokio::test]
async fn test_vote_deadline_expiry() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();

    // Create a vote with 1 second deadline
    let vote_info = agent_a
        .create_vote(
            "lobby",
            "Quick vote",
            None,
            vec!["Yes".to_string(), "No".to_string()],
            Some(1), // 1 second deadline
        )
        .await
        .unwrap();

    // Only agent A votes, agent B doesn't
    agent_a.cast_vote(&vote_info.vote_id, 0).await.unwrap();

    // Wait for deadline to expire + margin
    // Drain events until VoteResult arrives
    let result_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::VoteResult {
                    return event;
                }
            }
        }
    })
    .await
    .expect("VoteResult should arrive after deadline");

    // Should show 1 vote total (agent A voted, agent B didn't)
    let total = result_event
        .frame
        .payload
        .get("total_votes")
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(total, 1);
}

#[tokio::test]
async fn test_already_voted_rejected() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let vote_info = agent_a
        .create_vote(
            "lobby",
            "Test double vote",
            None,
            vec!["Yes".to_string(), "No".to_string()],
            None,
        )
        .await
        .unwrap();

    // Vote once
    agent_a.cast_vote(&vote_info.vote_id, 0).await.unwrap();

    // Try to vote again -- should fail
    let result = agent_a.cast_vote(&vote_info.vote_id, 1).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_option_does_not_consume_ballot() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent = connect_agent(&addr, &key, "agent-a").await;
    agent.join_room("lobby").await.unwrap();

    let vote_info = agent
        .create_vote(
            "lobby",
            "Invalid option should not consume ballot",
            None,
            vec!["Yes".to_string(), "No".to_string()],
            None,
        )
        .await
        .unwrap();

    let invalid = agent.cast_vote(&vote_info.vote_id, 99).await;
    assert!(invalid.is_err());

    let valid = agent.cast_vote(&vote_info.vote_id, 0).await.unwrap();
    assert_eq!(valid.get("votes_cast").unwrap().as_u64().unwrap(), 1);
}

// --- Election tests ---

#[tokio::test]
async fn test_leader_election() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();

    // Start election
    let resp = agent_a.elect_leader("lobby").await.unwrap();
    assert!(resp.get("candidates").is_some());

    // Wait for ElectionStarted then LeaderElected (after 2 second window)
    let leader_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::LeaderElected {
                    return event;
                }
            }
        }
    })
    .await
    .expect("LeaderElected should arrive after 2s window");

    let leader_id = leader_event
        .frame
        .payload
        .get("leader_id")
        .unwrap()
        .as_str()
        .unwrap();
    // Leader should be one of the two agents
    assert!(leader_id == agent_a.agent_id || leader_id == agent_b.agent_id);
}

#[tokio::test]
async fn test_leader_decision() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;

    // Create a room and be the only member
    let room = agent_a
        .create_room("decision-room", None, None)
        .await
        .unwrap();
    agent_a.join_room(&room.room_id).await.unwrap();

    let mut events_a = agent_a.subscribe();

    // Start election (with just one candidate, they'll be elected)
    agent_a.elect_leader(&room.room_id).await.unwrap();

    // Wait for leader elected
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::LeaderElected {
                    return event;
                }
            }
        }
    })
    .await
    .expect("Should be elected as sole candidate");

    // Issue a decision
    let resp = agent_a
        .send_decision(
            &room.room_id,
            "We'll go with Approach A",
            serde_json::json!({}),
        )
        .await
        .unwrap();
    assert!(resp.get("message_id").is_some());

    // Should receive DecisionMade event
    let decision_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::DecisionMade {
                    return event;
                }
            }
        }
    })
    .await
    .expect("DecisionMade should be broadcast");

    assert_eq!(
        decision_event
            .frame
            .payload
            .get("content")
            .unwrap()
            .as_str()
            .unwrap(),
        "We'll go with Approach A"
    );
}

#[tokio::test]
async fn test_non_leader_decision_rejected() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    agent_a.join_room("lobby").await.unwrap();

    // Try to issue a decision without being leader
    let result = agent_a
        .send_decision("lobby", "I decide this!", serde_json::json!({}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_decline_election() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_a = agent_a.subscribe();

    // Start election
    agent_a.elect_leader("lobby").await.unwrap();

    // Agent A declines
    agent_a.decline_election("lobby").await.unwrap();

    // Wait for LeaderElected -- should be agent B (the only remaining candidate)
    let leader_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events_a.recv().await {
                if event.frame.frame_type == FrameType::LeaderElected {
                    return event;
                }
            }
        }
    })
    .await
    .expect("LeaderElected should arrive");

    let leader_id = leader_event
        .frame
        .payload
        .get("leader_id")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(leader_id, agent_b.agent_id);
}

// --- Presence tests ---

#[tokio::test]
async fn test_presence_default_idle_on_register() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent = connect_agent(&addr, &key, "agent-a").await;
    let agents = agent.list_agents(None).await.unwrap();
    let me = agents.iter().find(|a| a.name == "agent-a").unwrap();

    assert_eq!(me.status.as_deref(), Some("idle"));
    assert_eq!(me.status_detail, None);
    assert_eq!(me.progress, None);
}

#[tokio::test]
async fn test_set_presence_updates_agent_info() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent = connect_agent(&addr, &key, "agent-a").await;

    // Set to working with detail and progress
    agent
        .set_presence("working", Some("reviewing section 3"), Some(57))
        .await
        .unwrap();

    let agents = agent.list_agents(None).await.unwrap();
    let me = agents.iter().find(|a| a.name == "agent-a").unwrap();
    assert_eq!(me.status.as_deref(), Some("working"));
    assert_eq!(me.status_detail.as_deref(), Some("reviewing section 3"));
    assert_eq!(me.progress, Some(57));

    // Thinking is a valid presence state and can describe the current thought.
    agent
        .set_presence("thinking", Some("considering option B"), None)
        .await
        .unwrap();

    let agents = agent.list_agents(None).await.unwrap();
    let me = agents.iter().find(|a| a.name == "agent-a").unwrap();
    assert_eq!(me.status.as_deref(), Some("thinking"));
    assert_eq!(me.status_detail.as_deref(), Some("considering option B"));
    assert_eq!(me.progress, None);

    // Set to waiting (clears detail and progress)
    agent.set_presence("waiting", None, None).await.unwrap();

    let agents = agent.list_agents(None).await.unwrap();
    let me = agents.iter().find(|a| a.name == "agent-a").unwrap();
    assert_eq!(me.status.as_deref(), Some("waiting"));
    assert_eq!(me.status_detail, None);
    assert_eq!(me.progress, None);
}

#[tokio::test]
async fn test_set_presence_invalid_status_rejected() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent = connect_agent(&addr, &key, "agent-a").await;
    let result = agent.set_presence("dancing", None, None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_presence_update_broadcast_to_room() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    let mut events_b = agent_b.subscribe();

    // Agent A sets presence
    agent_a
        .set_presence("working", Some("fixing bug #42"), Some(75))
        .await
        .unwrap();

    // Agent B should receive PresenceUpdate
    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(e) = events_b.recv().await {
                if e.frame.frame_type == FrameType::PresenceUpdate {
                    return e;
                }
            }
        }
    })
    .await
    .expect("agent-b should receive PresenceUpdate");

    assert_eq!(
        event
            .frame
            .payload
            .get("agent_name")
            .unwrap()
            .as_str()
            .unwrap(),
        "agent-a"
    );
    assert_eq!(
        event.frame.payload.get("status").unwrap().as_str().unwrap(),
        "working"
    );
    assert_eq!(
        event
            .frame
            .payload
            .get("status_detail")
            .unwrap()
            .as_str()
            .unwrap(),
        "fixing bug #42"
    );
    assert_eq!(
        event
            .frame
            .payload
            .get("progress")
            .unwrap()
            .as_u64()
            .unwrap(),
        75
    );
}

// --- Multi-agent coordination scenario ---

/// Full 3-agent workflow: discuss → vote → elect leader → decision.
/// Model is configurable via COWCHAT_MODEL env var (default: claude-opus-4.6).
#[tokio::test]
async fn test_three_agent_task_coordination() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    // --- Setup: connect 3 agents with model capability ---
    let architect = connect_agent_with_model(&addr, &key, "architect").await;
    let backend = connect_agent_with_model(&addr, &key, "backend-dev").await;
    let frontend = connect_agent_with_model(&addr, &key, "frontend-dev").await;

    // Verify model capability was registered
    let agents = architect.list_agents(None).await.unwrap();
    let model = std::env::var("COWCHAT_MODEL").unwrap_or_else(|_| "claude-opus-4.6".to_string());
    for agent in &agents {
        assert!(
            agent.capabilities.contains(&format!("model:{}", model)),
            "agent {} missing model capability",
            agent.name
        );
    }

    // --- Step 1: Create project room and join ---
    let room = architect
        .create_room("api-design", Some("API design coordination"), None)
        .await
        .unwrap();
    let room_id = room.room_id.clone();

    architect.join_room(&room_id).await.unwrap();
    backend.join_room(&room_id).await.unwrap();
    frontend.join_room(&room_id).await.unwrap();

    // Subscribe to events on all agents
    let mut events_arch = architect.subscribe();
    let mut events_back = backend.subscribe();
    let mut events_front = frontend.subscribe();

    // --- Step 2: Discussion ---
    architect
        .send_message(
            &room_id,
            "We need to pick an API style for the new service. Options are REST, GraphQL, or gRPC.",
            None,
            vec![],
        )
        .await
        .unwrap();

    backend
        .send_message(
            &room_id,
            "I prefer gRPC for inter-service communication - better performance and type safety.",
            None,
            vec![],
        )
        .await
        .unwrap();

    frontend
        .send_message(
            &room_id,
            "REST is more practical - easier to debug, better tooling, broader ecosystem.",
            None,
            vec![],
        )
        .await
        .unwrap();

    // --- Step 3: Sealed vote ---
    let vote_info = architect
        .create_vote(
            &room_id,
            "API style for the new service",
            Some("Choose the API paradigm for the user-facing service"),
            vec![
                "REST".to_string(),
                "GraphQL".to_string(),
                "gRPC".to_string(),
            ],
            None, // No deadline -- closes when all 3 vote
        )
        .await
        .unwrap();

    assert_eq!(vote_info.eligible_voters, 3);

    // All agents should receive VoteCreated
    let vote_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(e) = events_back.recv().await {
                if e.frame.frame_type == FrameType::VoteCreated {
                    return e;
                }
            }
        }
    })
    .await
    .expect("backend should receive VoteCreated");
    assert_eq!(
        vote_event
            .frame
            .payload
            .get("title")
            .unwrap()
            .as_str()
            .unwrap(),
        "API style for the new service"
    );

    // --- Step 4: Cast sealed ballots ---
    // architect votes REST (index 0)
    let resp = architect.cast_vote(&vote_info.vote_id, 0).await.unwrap();
    assert_eq!(resp.get("votes_cast").unwrap().as_u64().unwrap(), 1);

    // backend votes gRPC (index 2)
    let resp = backend.cast_vote(&vote_info.vote_id, 2).await.unwrap();
    assert_eq!(resp.get("votes_cast").unwrap().as_u64().unwrap(), 2);

    // Verify vote is still sealed -- status shows count but no ballots
    let status = frontend.get_vote_status(&vote_info.vote_id).await.unwrap();
    assert_eq!(status.votes_cast, 2);
    assert_eq!(status.eligible_voters, 3);

    // frontend votes REST (index 0) -- this triggers the result
    frontend.cast_vote(&vote_info.vote_id, 0).await.unwrap();

    // --- Step 5: Verify vote results ---
    let result_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(e) = events_arch.recv().await {
                if e.frame.frame_type == FrameType::VoteResult {
                    return e;
                }
            }
        }
    })
    .await
    .expect("architect should receive VoteResult");

    let tally = result_event
        .frame
        .payload
        .get("tally")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(tally.len(), 3);
    // REST=2, GraphQL=0, gRPC=1
    assert_eq!(
        tally[0].get("option_text").unwrap().as_str().unwrap(),
        "REST"
    );
    assert_eq!(tally[0].get("count").unwrap().as_u64().unwrap(), 2);
    assert_eq!(
        tally[1].get("option_text").unwrap().as_str().unwrap(),
        "GraphQL"
    );
    assert_eq!(tally[1].get("count").unwrap().as_u64().unwrap(), 0);
    assert_eq!(
        tally[2].get("option_text").unwrap().as_str().unwrap(),
        "gRPC"
    );
    assert_eq!(tally[2].get("count").unwrap().as_u64().unwrap(), 1);

    // All ballots revealed
    let ballots = result_event
        .frame
        .payload
        .get("ballots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(ballots.len(), 3);

    // --- Step 6: Leader election ---
    architect.elect_leader(&room_id).await.unwrap();

    // All agents receive ElectionStarted
    let election_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(e) = events_front.recv().await {
                if e.frame.frame_type == FrameType::ElectionStarted {
                    return e;
                }
            }
        }
    })
    .await
    .expect("frontend should receive ElectionStarted");
    let candidates = election_event
        .frame
        .payload
        .get("candidates")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(candidates.len(), 3);

    // backend declines -- they didn't want REST anyway
    backend.decline_election(&room_id).await.unwrap();

    // Wait for LeaderElected (after 2s window)
    let leader_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(e) = events_back.recv().await {
                if e.frame.frame_type == FrameType::LeaderElected {
                    return e;
                }
            }
        }
    })
    .await
    .expect("LeaderElected should arrive after 2s window");

    let leader_id = leader_event
        .frame
        .payload
        .get("leader_id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let leader_name = leader_event
        .frame
        .payload
        .get("leader_name")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Leader must be architect or frontend (backend declined)
    assert!(
        leader_id == architect.agent_id || leader_id == frontend.agent_id,
        "leader should be architect or frontend, got {}",
        leader_name
    );

    // --- Step 7: Leader issues decision ---
    // Figure out which client is the leader so we can send the decision
    let leader_client = if leader_id == architect.agent_id {
        &architect
    } else {
        &frontend
    };

    let decision_resp = leader_client
        .send_decision(
            &room_id,
            "We'll use REST with OpenAPI spec. The vote was clear: 2-1 in favor of REST.",
            serde_json::json!({"vote_id": vote_info.vote_id, "winning_option": "REST"}),
        )
        .await
        .unwrap();
    assert!(decision_resp.get("message_id").is_some());

    // Non-leader should NOT be able to issue a decision
    let non_leader = &backend;
    let reject = non_leader
        .send_decision(&room_id, "I override this!", serde_json::json!({}))
        .await;
    assert!(reject.is_err(), "non-leader decision should be rejected");

    // --- Step 8: Verify DecisionMade event ---
    // Drain events on backend (a non-leader observer) to find DecisionMade
    let decision_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(e) = events_back.recv().await {
                if e.frame.frame_type == FrameType::DecisionMade {
                    return e;
                }
            }
        }
    })
    .await
    .expect("backend should receive DecisionMade");

    assert_eq!(
        decision_event
            .frame
            .payload
            .get("content")
            .unwrap()
            .as_str()
            .unwrap(),
        "We'll use REST with OpenAPI spec. The vote was clear: 2-1 in favor of REST."
    );
    assert_eq!(
        decision_event
            .frame
            .payload
            .get("leader_name")
            .unwrap()
            .as_str()
            .unwrap(),
        leader_name
    );

    // --- Step 9: Verify history has the full discussion + decision ---
    let history = architect.get_history(&room_id, 50, None).await.unwrap();
    // Should have: 3 discussion messages + 1 decision message = 4
    assert!(
        history.len() >= 4,
        "expected at least 4 messages in history, got {}",
        history.len()
    );

    // First 3 are discussion
    assert_eq!(
        history[0].content,
        "We need to pick an API style for the new service. Options are REST, GraphQL, or gRPC."
    );
    assert_eq!(history[0].agent_name, "architect");
    assert_eq!(history[1].agent_name, "backend-dev");
    assert_eq!(history[2].agent_name, "frontend-dev");

    // Last message is the decision (has decision metadata)
    let decision_msg = history.last().unwrap();
    assert!(
        decision_msg.content.contains("REST with OpenAPI spec"),
        "decision should be in history"
    );
    assert_eq!(
        decision_msg.metadata.get("type").and_then(|v| v.as_str()),
        Some("decision"),
        "decision message should have type:decision metadata"
    );
}

// --- Turn token tests ---

async fn current_holder(client: &CowchatClient, room_id: &str) -> Option<String> {
    client.current_turn_holder(room_id).await.unwrap()
}

/// The turn token is advisory: the server publishes whose turn it is and advances
/// the token on every send, but a non-holder is allowed to speak (breaks the
/// "holder stuck in wait" deadlock). After any send, the token advances to the
/// member after the sender.
#[tokio::test]
async fn test_turn_token_is_advisory() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();

    // A joined first, so A holds the token.
    assert_eq!(
        current_holder(&agent_a, "lobby").await.as_deref(),
        Some(agent_a.agent_id.as_str())
    );

    // B, the non-holder, can still send. The send must succeed.
    agent_b
        .send_message("lobby", "B breaks in", None, vec![])
        .await
        .unwrap();

    // Token advances to the member after the sender (B). With two members that's A.
    assert_eq!(
        current_holder(&agent_a, "lobby").await.as_deref(),
        Some(agent_a.agent_id.as_str())
    );

    // A now sends; token advances to B.
    agent_a
        .send_message("lobby", "A responds", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&agent_a, "lobby").await.as_deref(),
        Some(agent_b.agent_id.as_str())
    );

    // And A can immediately send again — no enforcement.
    agent_a
        .send_message("lobby", "A again, out of turn", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&agent_a, "lobby").await.as_deref(),
        Some(agent_b.agent_id.as_str())
    );
}

/// If the turn holder disconnects, the remaining members receive a `turn_changed`
/// event with reason="disconnected" and the next member in join order becomes the holder.
#[tokio::test]
async fn test_turn_advances_on_disconnect() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let agent_a = connect_agent(&addr, &key, "agent-a").await;
    let agent_b = connect_agent(&addr, &key, "agent-b").await;

    agent_a.join_room("lobby").await.unwrap();
    agent_b.join_room("lobby").await.unwrap();
    let a_id = agent_a.agent_id.clone();
    let b_id = agent_b.agent_id.clone();

    // A holds.
    assert_eq!(
        current_holder(&agent_b, "lobby").await.as_deref(),
        Some(a_id.as_str())
    );

    let mut events_b = agent_b.subscribe();

    // A disconnects.
    drop(agent_a);

    // B should see a turn_changed event with reason=disconnected and holder=B.
    let evt = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let e = events_b.recv().await.unwrap();
            if e.frame.frame_type == FrameType::TurnChanged
                && e.frame.payload.get("reason").and_then(|v| v.as_str()) == Some("disconnected")
            {
                return e;
            }
        }
    })
    .await
    .expect("agent B should see turn_changed reason=disconnected");

    assert_eq!(
        evt.frame
            .payload
            .get("current_turn_holder")
            .and_then(|v| v.as_str()),
        Some(b_id.as_str())
    );

    // And B can now send.
    agent_b
        .send_message("lobby", "B speaks after A disconnects", None, vec![])
        .await
        .unwrap();
}

/// With three agents, the token rotates round-robin in join order.
#[tokio::test]
async fn test_join_order_is_round_robin() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let a = connect_agent(&addr, &key, "a").await;
    let b = connect_agent(&addr, &key, "b").await;
    let c = connect_agent(&addr, &key, "c").await;

    a.join_room("lobby").await.unwrap();
    b.join_room("lobby").await.unwrap();
    c.join_room("lobby").await.unwrap();

    let a_id = a.agent_id.clone();
    let b_id = b.agent_id.clone();
    let c_id = c.agent_id.clone();

    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(a_id.as_str())
    );

    a.send_message("lobby", "from a", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(b_id.as_str())
    );

    b.send_message("lobby", "from b", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(c_id.as_str())
    );

    c.send_message("lobby", "from c", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(a_id.as_str())
    );

    // And one more loop to confirm rotation is stable.
    a.send_message("lobby", "from a again", None, vec![])
        .await
        .unwrap();
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(b_id.as_str())
    );
}

// --- Thinking pulse tests ---

/// Thinking is broadcast to other room members as a `thinking` event, persists
/// to history with `metadata.type = "thinking"`, and does NOT advance the turn
/// token even when the sender holds it.
#[tokio::test]
async fn test_thinking_does_not_advance_token() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let a = connect_agent(&addr, &key, "agent-a").await;
    let b = connect_agent(&addr, &key, "agent-b").await;

    a.join_room("lobby").await.unwrap();
    b.join_room("lobby").await.unwrap();
    let a_id = a.agent_id.clone();

    // A holds the token (joined first).
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(a_id.as_str())
    );

    let mut events_b = b.subscribe();

    // A thinks out loud while holding the token.
    a.thinking("lobby", "reading the spec, ~30s").await.unwrap();

    // Token is unchanged — thinking doesn't pass the turn.
    assert_eq!(
        current_holder(&a, "lobby").await.as_deref(),
        Some(a_id.as_str())
    );

    // B receives it as a `thinking` event, not `message_received`.
    let evt = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let e = events_b.recv().await.unwrap();
            if e.frame.frame_type == FrameType::Thinking {
                return e;
            }
            assert_ne!(
                e.frame.frame_type,
                FrameType::MessageReceived,
                "thinking must not be broadcast as message_received"
            );
        }
    })
    .await
    .expect("B should receive a Thinking event");
    assert_eq!(
        evt.frame.payload.get("content").and_then(|v| v.as_str()),
        Some("reading the spec, ~30s")
    );
    assert_eq!(
        evt.frame
            .payload
            .get("metadata")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("thinking")
    );
}

/// `wait` with `--since-seq` returns a message that was sent BEFORE the wait
/// started, closing the race where a peer's reply lands between two waits and
/// is silently missed. Without `--since-seq` the call returns only future
/// messages (existing behavior).
#[tokio::test]
async fn test_wait_since_seq_catches_backlog() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let sender = connect_agent(&addr, &key, "sender").await;
    let waiter = connect_agent(&addr, &key, "waiter").await;

    sender.join_room("lobby").await.unwrap();
    waiter.join_room("lobby").await.unwrap();

    // Send first — before waiter calls wait.
    sender
        .send_message("lobby", "early", None, vec![])
        .await
        .unwrap();

    // Plain wait (no since_seq) must NOT see the backlog. Short timeout = expect None.
    let none = waiter.wait_for_message("lobby", 1, None).await.unwrap();
    assert!(none.is_none(), "plain wait should not see backlog");

    // With since_seq=0 the wait returns the oldest chat message with seq > 0.
    let got = waiter
        .wait_for_message("lobby", 2, Some(0))
        .await
        .unwrap()
        .expect("wait with since_seq should return the backlog");
    assert_eq!(got.content, "early");

    // Calling again with since_seq = got.seq drains the next backlog message
    // (none here) and blocks; a fresh send arrives and is returned.
    let next_msg = tokio::spawn({
        let waiter_id = waiter.agent_id.clone();
        let sender_clone_addr = addr.clone();
        let sender_clone_key = key.clone();
        let last_seq = got.seq;
        async move {
            // Give the waiter a moment to subscribe + check backlog (empty), then send.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let s = connect_agent(&sender_clone_addr, &sender_clone_key, "sender2").await;
            s.join_room("lobby").await.unwrap();
            s.send_message("lobby", "live", None, vec![]).await.unwrap();
            (waiter_id, last_seq)
        }
    });
    let (_, last_seq) = next_msg.await.unwrap();
    let live = waiter
        .wait_for_message("lobby", 3, Some(last_seq))
        .await
        .unwrap()
        .expect("wait should return the live message");
    assert_eq!(live.content, "live");
}

/// Sending with `metadata.kind` round-trips through history and is queryable.
/// This is the primitive that the CLI's `send --kind` and `history --kind`
/// commands rely on; there's no server-side filtering, just a client-side
/// convention that needs to survive persistence.
#[tokio::test]
async fn test_kind_metadata_roundtrips_through_history() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let a = connect_agent(&addr, &key, "agent-a").await;
    a.join_room("lobby").await.unwrap();

    a.send_message_with_metadata(
        "lobby",
        "review please",
        None,
        vec![],
        serde_json::json!({"kind": "review_request"}),
    )
    .await
    .unwrap();
    a.send_message_with_metadata(
        "lobby",
        "fyi: pushed more commits",
        None,
        vec![],
        serde_json::json!({"kind": "checkpoint"}),
    )
    .await
    .unwrap();
    a.send_message("lobby", "untagged note", None, vec![])
        .await
        .unwrap();

    let hist = a.get_history("lobby", 50, None).await.unwrap();
    let kinds: Vec<Option<&str>> = hist
        .iter()
        .map(|m| m.metadata.get("kind").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![Some("review_request"), Some("checkpoint"), None,]
    );

    // Client-side filter on metadata.kind matches what the CLI does.
    let only_requests: Vec<_> = hist
        .iter()
        .filter(|m| m.metadata.get("kind").and_then(|v| v.as_str()) == Some("review_request"))
        .collect();
    assert_eq!(only_requests.len(), 1);
    assert_eq!(only_requests[0].content, "review please");
}

/// `wait_for_message_loop` keeps re-polling internally until a message arrives,
/// even when each inner iteration times out — so a peer's message that lands
/// AFTER the first inner timeout is still caught. This is what the CLI's
/// `wait --loop` exposes to agents; without it Codex misses any Claude post
/// that arrives more than `timeout` seconds after the wait started.
#[tokio::test]
async fn test_wait_loop_picks_up_message_after_inner_timeout() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let sender = connect_agent(&addr, &key, "sender").await;
    let waiter = connect_agent(&addr, &key, "waiter").await;

    sender.join_room("lobby").await.unwrap();
    waiter.join_room("lobby").await.unwrap();

    // Sender posts ~1.5s in — well past one inner iteration's 1s budget.
    let sender_clone_addr = addr.clone();
    let sender_clone_key = key.clone();
    let post_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let s = connect_agent(&sender_clone_addr, &sender_clone_key, "sender2").await;
        s.join_room("lobby").await.unwrap();
        s.send_message("lobby", "late but caught", None, vec![])
            .await
            .unwrap();
    });

    // Loop with a deliberately short inner timeout so the first iteration MUST
    // time out before the sender's message arrives.
    let got = waiter
        .wait_for_message_loop("lobby", 1, Some(0))
        .await
        .unwrap();
    assert_eq!(got.content, "late but caught");

    post_task.await.unwrap();
}

/// A late-joining client should be able to retrieve prior thinking pulses via
/// `get_history` — that's the whole point of persisting them.
#[tokio::test]
async fn test_thinking_appears_in_history() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let a = connect_agent(&addr, &key, "agent-a").await;
    a.join_room("lobby").await.unwrap();

    a.send_message("lobby", "starting review", None, vec![])
        .await
        .unwrap();
    a.thinking("lobby", "considering option B").await.unwrap();
    a.thinking("lobby", "settled on option A").await.unwrap();
    a.send_message("lobby", "ok, going with A", None, vec![])
        .await
        .unwrap();

    // Late-joining client sees the full timeline including thoughts.
    let late = connect_agent(&addr, &key, "agent-late").await;
    late.join_room("lobby").await.unwrap();
    let history = late.get_history("lobby", 50, None).await.unwrap();

    assert_eq!(
        history.len(),
        4,
        "expected 4 entries (2 messages + 2 thoughts)"
    );

    let kinds: Vec<&str> = history
        .iter()
        .map(|m| {
            m.metadata
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("chat")
        })
        .collect();
    assert_eq!(kinds, vec!["chat", "thinking", "thinking", "chat"]);
    assert_eq!(history[1].content, "considering option B");
    assert_eq!(history[2].content, "settled on option A");
}

// --- Webhook subscription tests ---

#[derive(Clone, Debug)]
struct RecordedRequest {
    body: String,
    webhook_id: String,
    webhook_timestamp: String,
    webhook_signature: String,
}

/// Spawn an axum mini-receiver on a random localhost port that captures every
/// POST and returns `status_code`. Returns (base_url, captured_requests).
async fn spawn_test_receiver(
    status_code: u16,
) -> (
    String,
    std::sync::Arc<tokio::sync::Mutex<Vec<RecordedRequest>>>,
) {
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;

    let captured: std::sync::Arc<tokio::sync::Mutex<Vec<RecordedRequest>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let handler = move |headers: HeaderMap, body: Bytes| {
        let captured = captured_clone.clone();
        async move {
            let webhook_id = headers
                .get("webhook-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let webhook_timestamp = headers
                .get("webhook-timestamp")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let webhook_signature = headers
                .get("webhook-signature")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let body_str = String::from_utf8_lossy(&body).to_string();
            captured.lock().await.push(RecordedRequest {
                body: body_str,
                webhook_id,
                webhook_timestamp,
                webhook_signature,
            });
            StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK)
        }
    };

    let app = Router::new().route("/hook", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (format!("http://{}/hook", addr), captured)
}

async fn spawn_redirect_receiver() -> (
    String,
    std::sync::Arc<tokio::sync::Mutex<Vec<RecordedRequest>>>,
) {
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;

    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let handler = move |headers: HeaderMap, body: Bytes| {
        let captured = captured_clone.clone();
        async move {
            captured.lock().await.push(RecordedRequest {
                body: String::from_utf8_lossy(&body).to_string(),
                webhook_id: headers
                    .get("webhook-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string(),
                webhook_timestamp: String::new(),
                webhook_signature: String::new(),
            });
            (
                StatusCode::FOUND,
                [("location", "http://169.254.169.254/latest/meta-data")],
            )
                .into_response()
        }
    };
    let app = Router::new().route("/hook", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/hook"), captured)
}

/// Verify a recorded request's Standard Webhooks signature matches the body.
fn verify_signature(req: &RecordedRequest, secret: &str) -> bool {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let Some(sig_b64) = req.webhook_signature.strip_prefix("v1,") else {
        return false;
    };
    let Ok(expected) = STANDARD.decode(sig_b64) else {
        return false;
    };
    let signed = format!("{}.{}.{}", req.webhook_id, req.webhook_timestamp, req.body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(signed.as_bytes());
    mac.verify_slice(&expected).is_ok()
}

async fn wait_for_deliveries(
    captured: &std::sync::Arc<tokio::sync::Mutex<Vec<RecordedRequest>>>,
    expected: usize,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if captured.lock().await.len() >= expected {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    captured.lock().await.len() >= expected
}

#[tokio::test]
async fn test_webhook_delivers_signed_post() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url, captured) = spawn_test_receiver(200).await;

    let agent = connect_agent(&addr, &key, "agent").await;
    agent.join_room("lobby").await.unwrap();

    let sub = agent
        .create_subscription(
            "lobby",
            &hook_url,
            "topsecret",
            vec![],
            None,
            None,
            false,
            Some(0),
        )
        .await
        .expect("subscribe");
    assert_eq!(sub.status, "active");

    agent
        .send_message("lobby", "hello webhook", None, vec![])
        .await
        .unwrap();

    assert!(
        wait_for_deliveries(&captured, 1, Duration::from_secs(5)).await,
        "expected exactly 1 delivery within 5s"
    );

    let recorded = captured.lock().await;
    let req = &recorded[0];
    assert!(!req.webhook_id.is_empty(), "webhook-id header set");
    assert!(!req.webhook_timestamp.is_empty(), "webhook-timestamp set");
    assert!(req.webhook_signature.starts_with("v1,"), "v1 sig prefix");
    assert!(verify_signature(req, "topsecret"), "signature verifies");

    let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["type"], "cowchat.message.created");
    assert_eq!(body["subscription_id"], sub.subscription_id);
    assert_eq!(body["room_id"], "lobby");
    assert_eq!(body["message"]["content"], "hello webhook");
}

#[tokio::test]
async fn test_webhook_filter_only_kind() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url, captured) = spawn_test_receiver(200).await;

    let agent = connect_agent(&addr, &key, "agent").await;
    agent.join_room("lobby").await.unwrap();

    agent
        .create_subscription(
            "lobby",
            &hook_url,
            "s",
            vec!["verdict".into()],
            None,
            None,
            false,
            Some(0),
        )
        .await
        .unwrap();

    // Two non-matching, one matching, one non-matching.
    agent
        .send_message_with_metadata(
            "lobby",
            "noise 1",
            None,
            vec![],
            serde_json::json!({"kind": "checkpoint"}),
        )
        .await
        .unwrap();
    agent
        .send_message_with_metadata(
            "lobby",
            "the verdict",
            None,
            vec![],
            serde_json::json!({"kind": "verdict"}),
        )
        .await
        .unwrap();
    agent
        .send_message_with_metadata(
            "lobby",
            "noise 2",
            None,
            vec![],
            serde_json::json!({"kind": "fyi"}),
        )
        .await
        .unwrap();

    assert!(
        wait_for_deliveries(&captured, 1, Duration::from_secs(5)).await,
        "expected exactly 1 delivery"
    );
    // Give time for any extra (non-)deliveries to NOT arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let recorded = captured.lock().await;
    assert_eq!(recorded.len(), 1, "kind filter should drop the other two");
    let body: serde_json::Value = serde_json::from_str(&recorded[0].body).unwrap();
    assert_eq!(body["message"]["content"], "the verdict");
}

#[tokio::test]
async fn test_webhook_retries_on_5xx() {
    // Receiver returns 500 forever. Verify the worker actually retries the
    // delivery (we see >=2 attempts within ~5s, matching the [1s, 4s, ...]
    // backoff schedule). The full "permanent fail after 5 retries" path is too
    // slow to test directly (>250s) — we just confirm retries happen.
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url, captured) = spawn_test_receiver(500).await;

    let agent = connect_agent(&addr, &key, "agent").await;
    agent.join_room("lobby").await.unwrap();
    agent
        .create_subscription("lobby", &hook_url, "s", vec![], None, None, false, Some(0))
        .await
        .unwrap();
    agent
        .send_message("lobby", "will fail", None, vec![])
        .await
        .unwrap();

    // First attempt is immediate; first retry is 1s later. By 3s we should see
    // at least the initial + 1 retry.
    let saw_retries = wait_for_deliveries(&captured, 2, Duration::from_secs(5)).await;
    assert!(
        saw_retries,
        "expected at least 2 attempts (1 initial + 1 retry) on a 500 receiver"
    );

    // And the row should not have been cleared (the sub stays active during
    // retry; the delivery row stays queued).
    let subs = agent.list_subscriptions(Some("lobby")).await.unwrap();
    assert!(
        subs.iter().any(|s| s.status == "active"),
        "subscription should still be active during the retry window"
    );
}

#[tokio::test]
async fn test_webhook_does_not_follow_redirects() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url, captured) = spawn_redirect_receiver().await;
    let agent = connect_agent(&addr, &key, "redirect-test").await;
    agent.join_room("lobby").await.unwrap();
    agent
        .create_subscription("lobby", &hook_url, "s", vec![], None, None, false, Some(0))
        .await
        .unwrap();
    agent
        .send_message("lobby", "do not redirect", None, vec![])
        .await
        .unwrap();
    assert!(
        wait_for_deliveries(&captured, 2, Duration::from_secs(5)).await,
        "a 302 must be treated as a failed attempt and retried at the pinned origin"
    );
}

#[tokio::test]
async fn test_webhook_backfill_on_create() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url, captured) = spawn_test_receiver(200).await;

    let agent = connect_agent(&addr, &key, "agent").await;
    agent.join_room("lobby").await.unwrap();

    // Pre-existing messages, BEFORE the subscription exists.
    agent
        .send_message("lobby", "past 1", None, vec![])
        .await
        .unwrap();
    agent
        .send_message("lobby", "past 2", None, vec![])
        .await
        .unwrap();

    // Now create the sub with since_seq=0 — both past messages should be replayed.
    agent
        .create_subscription("lobby", &hook_url, "s", vec![], None, None, false, Some(0))
        .await
        .unwrap();

    assert!(
        wait_for_deliveries(&captured, 2, Duration::from_secs(5)).await,
        "expected backfill of 2 messages"
    );
    let recorded = captured.lock().await;
    let contents: Vec<&str> = recorded
        .iter()
        .map(|r| {
            let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
            v["message"]["content"].as_str().unwrap().to_string().leak() as &str
        })
        .collect();
    assert_eq!(contents, vec!["past 1", "past 2"]);
}

#[tokio::test]
async fn test_webhook_backfill_is_not_capped_at_one_thousand() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    {
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        let tx = conn.transaction().unwrap();
        for seq in 1..=1005i64 {
            tx.execute(
                "INSERT INTO messages
                 (message_id, room_id, agent_id, agent_name, content, metadata, seq)
                 VALUES (?1, 'lobby', 'seed', 'Seed', 'backfill', '{}', ?2)",
                rusqlite::params![format!("backfill-{seq}"), seq],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    let client = connect_agent(&addr, &key, "subscriber").await;
    let subscription = client
        .create_subscription(
            "lobby",
            "http://127.0.0.1:9/hook",
            "secret",
            vec![],
            None,
            None,
            true,
            Some(0),
        )
        .await
        .unwrap();
    let count: i64 = rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM subscription_deliveries WHERE subscription_id = ?1",
            [&subscription.subscription_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1005,
        "backfill must page through the complete history"
    );
}

#[tokio::test]
async fn test_webhook_idempotent_enqueue_per_sub_per_seq() {
    // Same message can be delivered to TWO different subs, but the same
    // (subscription_id, message_seq) can't be queued twice — the unique index
    // prevents it. This guards against a race where two enqueue attempts land
    // simultaneously.
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let (hook_url_a, captured_a) = spawn_test_receiver(200).await;
    let (hook_url_b, captured_b) = spawn_test_receiver(200).await;

    let agent = connect_agent(&addr, &key, "agent").await;
    agent.join_room("lobby").await.unwrap();

    agent
        .create_subscription(
            "lobby",
            &hook_url_a,
            "s",
            vec![],
            None,
            None,
            false,
            Some(0),
        )
        .await
        .unwrap();
    agent
        .create_subscription(
            "lobby",
            &hook_url_b,
            "s",
            vec![],
            None,
            None,
            false,
            Some(0),
        )
        .await
        .unwrap();

    agent
        .send_message("lobby", "one shared message", None, vec![])
        .await
        .unwrap();

    assert!(wait_for_deliveries(&captured_a, 1, Duration::from_secs(5)).await);
    assert!(wait_for_deliveries(&captured_b, 1, Duration::from_secs(5)).await);

    // Wait a beat to ensure no duplicates land.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        captured_a.lock().await.len(),
        1,
        "receiver A: exactly 1 delivery"
    );
    assert_eq!(
        captured_b.lock().await.len(),
        1,
        "receiver B: exactly 1 delivery"
    );
}

#[tokio::test]
async fn test_e2e_encrypted_room() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let secret = b"super-secret-room-key";

    // Two agents that share the pre-shared room secret.
    let mut alice = connect_agent(&addr, &key, "alice").await;
    alice.set_room_secret(secret);
    let mut bob = connect_agent(&addr, &key, "bob").await;
    bob.set_room_secret(secret);

    // Alice creates a persistent end-to-end encrypted room.
    let room = alice
        .create_room_with_options("vault", None, None, false, true)
        .await
        .unwrap();
    assert!(room.encrypted, "room should be marked encrypted");

    alice.join_room(&room.room_id).await.unwrap();
    bob.join_room(&room.room_id).await.unwrap();

    // Bob bookmarks the tip, Alice sends, Bob catches up + decrypts.
    let tip = bob.room_tip(&room.room_id).await.unwrap();
    let sent = alice
        .send_message(&room.room_id, "the eagle lands at dawn", None, vec![])
        .await
        .unwrap();
    // Sender sees their own plaintext echoed back (decrypted locally).
    assert_eq!(sent.content, "the eagle lands at dawn");

    // Bob receives and transparently decrypts to plaintext.
    let received = bob
        .wait_for_message(&room.room_id, 3, Some(tip))
        .await
        .unwrap()
        .expect("bob receives the message");
    assert_eq!(received.content, "the eagle lands at dawn");

    // The server stored only ciphertext — it cannot read the plaintext.
    let db_path = tmp.path().join("test.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT content FROM messages WHERE room_id = ?1 ORDER BY seq DESC LIMIT 1",
            [&room.room_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        stored.starts_with("cow1:"),
        "stored content should be a ciphertext blob, got: {stored}"
    );
    assert!(
        !stored.contains("eagle"),
        "plaintext must not be stored server-side"
    );

    // A third agent WITHOUT the key sees ciphertext, not plaintext.
    let outsider = connect_agent(&addr, &key, "outsider").await;
    outsider.join_room(&room.room_id).await.unwrap();
    let history = outsider.get_history(&room.room_id, 50, None).await.unwrap();
    let last = history.last().expect("a message exists in history");
    assert!(
        last.content.starts_with("cow1:"),
        "keyless agent should see ciphertext, got: {}",
        last.content
    );

    // A keyless agent cannot silently downgrade the room by sending plaintext.
    let err = outsider
        .send_message(&room.room_id, "plaintext leak attempt", None, vec![])
        .await
        .unwrap_err();
    match err {
        cowchat_client::ClientError::Server { code, .. } => assert_eq!(
            code,
            cowchat_core::ErrorCode::PlaintextInEncryptedRoom,
            "plaintext send to an encrypted room must be rejected"
        ),
        other => panic!("expected PlaintextInEncryptedRoom, got {other:?}"),
    }
}

#[tokio::test]
async fn test_agent_id_takeover() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    // First connection claims a stable agent_id and joins a room.
    let a1 = CowchatClient::connect_tcp(&addr, &key, "stable", Some("stable-agent"), vec![])
        .await
        .unwrap();
    a1.join_room("lobby").await.unwrap();
    a1.send_message("lobby", "from a1", None, vec![])
        .await
        .unwrap();

    // A second connection with the SAME agent_id (still live) must take over
    // instead of being rejected with agent_id_taken — this is the rapid
    // re-use / reconnect-race case.
    let a2 = CowchatClient::connect_tcp(&addr, &key, "stable", Some("stable-agent"), vec![])
        .await
        .expect("second connect with same agent_id should take over, not fail");

    // The take-over inherits a1's room membership: a send to lobby succeeds only
    // if a2 is a member (the server rejects sends from non-members).
    a2.send_message("lobby", "from a2 after takeover", None, vec![])
        .await
        .expect("a2 should have inherited lobby membership via take-over");

    let hist = a2.get_history("lobby", 10, None).await.unwrap();
    assert!(
        hist.iter().any(|m| m.content == "from a2 after takeover"),
        "a2's post-takeover message should be in history"
    );

    // a2 is the live owner; dropping a1 (its cleanup) must NOT evict a2.
    drop(a1);
    sleep(Duration::from_millis(300)).await;
    a2.send_message("lobby", "a2 still alive", None, vec![])
        .await
        .expect("a2 must survive a1's delayed disconnect cleanup");
}

#[tokio::test]
async fn test_agent_id_takeover_is_bound_to_api_key() {
    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "second-valid-api-key";
    let db_path = tmp.path().join("test.db");
    rusqlite::Connection::open(&db_path)
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'test key b')",
            [key_b],
        )
        .unwrap();

    let owner = CowchatClient::connect_tcp(
        &addr,
        &key_a,
        "owner",
        Some("credential-bound-agent"),
        vec![],
    )
    .await
    .unwrap();
    owner.join_room("lobby").await.unwrap();

    let live_takeover = CowchatClient::connect_tcp(
        &addr,
        key_b,
        "attacker",
        Some("credential-bound-agent"),
        vec![],
    )
    .await;
    assert!(
        live_takeover.is_err(),
        "another key must not take over a live identity"
    );

    drop(owner);
    sleep(Duration::from_millis(300)).await;
    let stashed_takeover = CowchatClient::connect_tcp(
        &addr,
        key_b,
        "attacker",
        Some("credential-bound-agent"),
        vec![],
    )
    .await;
    assert!(
        stashed_takeover.is_err(),
        "another key must not reclaim a stashed identity"
    );

    CowchatClient::connect_tcp(
        &addr,
        &key_a,
        "owner",
        Some("credential-bound-agent"),
        vec![],
    )
    .await
    .expect("the owning key should still reclaim its stable identity");
}

#[tokio::test]
async fn test_agent_id_ownership_survives_server_restart() {
    let (handle, addr, key_a, tmp) = start_test_server().await;
    let owner =
        CowchatClient::connect_tcp(&addr, &key_a, "owner", Some("restart-owned-agent"), vec![])
            .await
            .unwrap();
    drop(owner);
    handle.abort();
    let _ = handle.await;

    let key_b = "restart-outsider-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'restart outsider')",
            [key_b],
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let restarted_addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    let restarted = CowchatServer::new(ServerConfig {
        socket_path: tmp.path().join("restart.sock"),
        tcp_addr: Some(restarted_addr.clone()),
        http_addr: None,
        db_path: tmp.path().join("test.db"),
        auth_key_path: tmp.path().join("auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
    })
    .unwrap();
    let restarted_handle = tokio::spawn(async move {
        let _ = restarted.run().await;
    });
    sleep(Duration::from_millis(100)).await;

    let stolen = CowchatClient::connect_tcp(
        &restarted_addr,
        key_b,
        "attacker",
        Some("restart-owned-agent"),
        vec![],
    )
    .await;
    assert!(stolen.is_err(), "identity ownership must survive restart");
    CowchatClient::connect_tcp(
        &restarted_addr,
        &key_a,
        "owner",
        Some("restart-owned-agent"),
        vec![],
    )
    .await
    .expect("the original credential must retain its identity after restart");
    restarted_handle.abort();
}

#[tokio::test]
async fn test_private_room_reads_and_creation_events_are_key_scoped() {
    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "private-room-outsider-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'outsider')",
            [key_b],
        )
        .unwrap();

    let owner = connect_agent(&addr, &key_a, "owner").await;
    let outsider = connect_agent(&addr, key_b, "outsider").await;
    let mut outsider_events = outsider.subscribe();
    let room = owner
        .create_room("key-private-room", None, None)
        .await
        .unwrap();
    owner.join_room(&room.room_id).await.unwrap();
    owner
        .send_message(&room.room_id, "private", None, vec![])
        .await
        .unwrap();
    let vote = owner
        .create_vote(
            &room.room_id,
            "private vote",
            None,
            vec!["a".into(), "b".into()],
            None,
        )
        .await
        .unwrap();

    assert!(outsider.get_history(&room.room_id, 10, None).await.is_err());
    assert!(outsider.room_tip(&room.room_id).await.is_err());
    assert!(outsider.room_info(&room.room_id).await.is_err());
    assert!(outsider.list_agents(Some(&room.room_id)).await.is_err());
    assert!(outsider.list_votes(&room.room_id, 10).await.is_err());
    assert!(outsider.get_vote_status(&vote.vote_id).await.is_err());

    let leaked_creation = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Ok(event) = outsider_events.recv().await {
                if event.frame.frame_type == FrameType::RoomCreated
                    && event.frame.payload.get("room_id").and_then(|v| v.as_str())
                        == Some(room.room_id.as_str())
                {
                    return true;
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        !leaked_creation,
        "private RoomCreated must not leak across API keys"
    );

    let doomed = owner
        .create_room("key-private-doomed", None, None)
        .await
        .unwrap();
    owner.join_room(&doomed.room_id).await.unwrap();
    owner.destroy_room(&doomed.room_id).await.unwrap();
    let leaked_destroy = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Ok(event) = outsider_events.recv().await {
                if event.frame.frame_type == FrameType::RoomDestroyed
                    && event.frame.payload.get("room_id").and_then(|v| v.as_str())
                        == Some(doomed.room_id.as_str())
                {
                    return true;
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        !leaked_destroy,
        "private RoomDestroyed must not leak across API keys"
    );
}

#[tokio::test]
async fn test_persistent_room_rename_is_authorized_scoped_and_reconnect_visible() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "rename-outsider-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'rename outsider')",
            [key_b],
        )
        .unwrap();

    let owner = CowchatClient::connect_tcp(&addr, &key_a, "owner", Some("rename-owner"), vec![])
        .await
        .unwrap();
    let same_key_peer = CowchatClient::connect_tcp(
        &addr,
        &key_a,
        "same-key-peer",
        Some("rename-same-key-peer"),
        vec![],
    )
    .await
    .unwrap();
    let outsider =
        CowchatClient::connect_tcp(&addr, key_b, "outsider", Some("rename-outsider"), vec![])
            .await
            .unwrap();
    let stashed =
        CowchatClient::connect_tcp(&addr, &key_a, "stashed", Some("rename-stashed"), vec![])
            .await
            .unwrap();

    let room = owner
        .create_room("Persistent Old", None, None)
        .await
        .unwrap();
    stashed.join_room(&room.room_id).await.unwrap();
    drop(stashed);
    for _ in 0..30 {
        if !owner
            .list_agents(None)
            .await
            .unwrap()
            .iter()
            .any(|agent| agent.agent_id == "rename-stashed")
        {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    let denied = same_key_peer
        .rename_room(&room.room_id, "Peer Attempt")
        .await
        .unwrap_err();
    match denied {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }
    let denied = outsider
        .rename_room(&room.room_id, "Outsider Attempt")
        .await
        .unwrap_err();
    match denied {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }
    let denied = owner.rename_room("lobby", "New Lobby").await.unwrap_err();
    match denied {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }

    let mut owner_events = owner.subscribe();
    let mut peer_events = same_key_peer.subscribe();
    let mut outsider_events = outsider.subscribe();
    let updated = owner
        .rename_room(&room.room_id, "  Persistent Renamed  ")
        .await
        .unwrap();
    assert_eq!(updated.room_id, room.room_id);
    assert_eq!(updated.name, "Persistent Renamed");
    assert!(
        updated.owner_key.is_none(),
        "owner key must stay off the wire"
    );

    for events in [&mut owner_events, &mut peer_events] {
        let event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.frame.frame_type == FrameType::RoomUpdated
                    && event.frame.payload["room_id"] == room.room_id
                {
                    break event.frame;
                }
            }
        })
        .await
        .expect("authorized clients receive room_updated");
        assert_eq!(event.payload["name"], "Persistent Renamed");
        assert!(event.payload.get("owner_key").is_none());
    }
    let leaked = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = outsider_events.recv().await.unwrap();
            if event.frame.frame_type == FrameType::RoomUpdated
                && event.frame.payload["room_id"] == room.room_id
            {
                break;
            }
        }
    })
    .await;
    assert!(
        leaked.is_err(),
        "private room_updated must not cross API keys"
    );

    let persisted = rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .query_row(
            "SELECT name FROM rooms WHERE room_id = ?1",
            [&room.room_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(persisted, "Persistent Renamed");
    assert_eq!(
        owner.room_info(&room.room_id).await.unwrap()["room"]["name"],
        "Persistent Renamed",
    );

    // A disconnected authorized client receives the metadata event before its
    // membership is restored, so a reconnecting UI cannot retain the old name.
    let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let register = Frame {
        id: Some("rename-reconnect".into()),
        reply_to: None,
        frame_type: FrameType::Register,
        payload: serde_json::to_value(RegisterPayload {
            key: key_a.clone(),
            agent_id: Some("rename-stashed".into()),
            name: "stashed".into(),
            capabilities: vec![],
            reconnect: true,
            protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
        })
        .unwrap(),
    };
    write_half
        .write_all(register.to_line().unwrap().as_bytes())
        .await
        .unwrap();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert_eq!(Frame::from_line(&line).unwrap().frame_type, FrameType::Ok);
    line.clear();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("replayed room_updated after register")
        .unwrap();
    let replayed = Frame::from_line(&line).unwrap();
    assert_eq!(replayed.frame_type, FrameType::RoomUpdated);
    assert_eq!(replayed.payload["room_id"], room.room_id);
    assert_eq!(replayed.payload["name"], "Persistent Renamed");
}

#[tokio::test]
async fn test_public_room_rename_and_name_invariants() {
    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "rename-public-observer-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'public observer')",
            [key_b],
        )
        .unwrap();
    let owner = CowchatClient::connect_tcp(&addr, &key_a, "owner", Some("public-renamer"), vec![])
        .await
        .unwrap();
    let observer = CowchatClient::connect_tcp(
        &addr,
        key_b,
        "observer",
        Some("rename-public-observer"),
        vec![],
    )
    .await
    .unwrap();

    let persistent = owner
        .create_room("Persistent Conflict", None, None)
        .await
        .unwrap();
    let denied_child = observer
        .create_room("Unauthorized Child", None, Some(&persistent.room_id))
        .await
        .unwrap_err();
    match denied_child {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }
    let missing_parent = owner
        .create_room("Dangling Child", None, Some("missing-parent"))
        .await
        .unwrap_err();
    match missing_parent {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::RoomNotFound)
        }
        other => panic!("expected room_not_found, got {other:?}"),
    }
    let public = owner
        .create_room_with_options("  Public Old  ", None, None, true, false)
        .await
        .unwrap();
    assert_eq!(public.name, "Public Old");

    let duplicate = owner
        .create_room("Public Old", None, None)
        .await
        .unwrap_err();
    match duplicate {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::RoomNameTaken)
        }
        other => panic!("expected room_name_taken, got {other:?}"),
    }
    let duplicate = owner
        .create_room("Persistent Conflict", None, None)
        .await
        .unwrap_err();
    match duplicate {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::RoomNameTaken)
        }
        other => panic!("expected room_name_taken, got {other:?}"),
    }

    for invalid in ["   ".to_string(), "bad\nname".to_string(), "🦀".repeat(101)] {
        let error = owner
            .rename_room(&public.room_id, &invalid)
            .await
            .unwrap_err();
        match error {
            cowchat_client::ClientError::Server { code, .. } => {
                assert_eq!(code, cowchat_core::ErrorCode::InvalidPayload)
            }
            other => panic!("expected invalid_payload, got {other:?}"),
        }
    }
    let conflict = owner
        .rename_room(&public.room_id, &persistent.name)
        .await
        .unwrap_err();
    match conflict {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::RoomNameTaken)
        }
        other => panic!("expected room_name_taken, got {other:?}"),
    }

    let mut observer_events = observer.subscribe();
    let updated = owner
        .rename_room(&public.room_id, "  Public Renamed  ")
        .await
        .unwrap();
    assert_eq!(updated.name, "Public Renamed");
    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = observer_events.recv().await.unwrap();
            if event.frame.frame_type == FrameType::RoomUpdated
                && event.frame.payload["room_id"] == public.room_id
            {
                break event.frame;
            }
        }
    })
    .await
    .expect("public room_updated reaches another API key");
    assert_eq!(event.payload["name"], "Public Renamed");
    assert!(owner
        .list_rooms(None)
        .await
        .unwrap()
        .iter()
        .any(|room| room.room_id == public.room_id && room.name == "Public Renamed"));

    for invalid in ["".to_string(), "bad\tname".to_string(), "界".repeat(101)] {
        let error = owner.create_room(&invalid, None, None).await.unwrap_err();
        match error {
            cowchat_client::ClientError::Server { code, .. } => {
                assert_eq!(code, cowchat_core::ErrorCode::InvalidPayload)
            }
            other => panic!("expected invalid_payload, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_explicit_room_destruction_is_creator_only_scoped_and_complete() {
    let (_handle, addr, key_a, tmp) = start_test_server().await;
    let key_b = "destroy-outsider-key";
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, 'outsider')",
            [key_b],
        )
        .unwrap();

    let owner = CowchatClient::connect_tcp(&addr, &key_a, "owner", Some("destroy-owner"), vec![])
        .await
        .unwrap();
    let same_key_peer = CowchatClient::connect_tcp(
        &addr,
        &key_a,
        "same-key-peer",
        Some("same-key-peer"),
        vec![],
    )
    .await
    .unwrap();
    let outsider =
        CowchatClient::connect_tcp(&addr, key_b, "outsider", Some("destroy-outsider"), vec![])
            .await
            .unwrap();

    let room = owner
        .create_room("explicit-destroy", None, None)
        .await
        .unwrap();
    assert_eq!(room.created_by.as_deref(), Some("destroy-owner"));
    assert!(
        room.owner_key.is_none(),
        "the bearer API key must be redacted from create_room responses"
    );
    let info = owner.room_info(&room.room_id).await.unwrap();
    let encoded_info = serde_json::to_string(&info).unwrap();
    assert!(!encoded_info.contains("owner_key"));
    assert!(!encoded_info.contains(&key_a));

    let denied = same_key_peer.destroy_room(&room.room_id).await.unwrap_err();
    match denied {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }
    let lobby_denied = owner.destroy_room("lobby").await.unwrap_err();
    match lobby_denied {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }

    owner.join_room(&room.room_id).await.unwrap();
    owner
        .send_message(&room.room_id, "delete me", None, vec![])
        .await
        .unwrap();
    let vote = owner
        .create_vote(
            &room.room_id,
            "delete vote",
            None,
            vec!["yes".into(), "no".into()],
            None,
        )
        .await
        .unwrap();
    owner
        .create_subscription(
            &room.room_id,
            "http://127.0.0.1:9/hook",
            "webhook-secret",
            vec![],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

    let mut owner_events = owner.subscribe();
    let mut peer_events = same_key_peer.subscribe();
    let mut outsider_events = outsider.subscribe();
    owner.destroy_room(&room.room_id).await.unwrap();

    for events in [&mut owner_events, &mut peer_events] {
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.frame.frame_type == FrameType::RoomDestroyed
                    && event
                        .frame
                        .payload
                        .get("room_id")
                        .and_then(|value| value.as_str())
                        == Some(room.room_id.as_str())
                {
                    break true;
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(observed, "authorized clients must receive room_destroyed");
    }

    let leaked = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Ok(event) = outsider_events.recv().await {
                if event.frame.frame_type == FrameType::RoomDestroyed
                    && event
                        .frame
                        .payload
                        .get("room_id")
                        .and_then(|value| value.as_str())
                        == Some(room.room_id.as_str())
                {
                    break true;
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(!leaked, "private destruction must not leak across API keys");

    assert!(owner.room_info(&room.room_id).await.is_err());
    assert!(owner.get_history(&room.room_id, 10, None).await.is_err());
    assert!(owner.get_vote_status(&vote.vote_id).await.is_err());
    let missing = owner.destroy_room(&room.room_id).await.unwrap_err();
    match missing {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::RoomNotFound)
        }
        other => panic!("expected room_not_found, got {other:?}"),
    }
    assert!(owner
        .list_subscriptions(Some(&room.room_id))
        .await
        .unwrap()
        .is_empty());

    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    for (table, predicate) in [
        ("rooms", "room_id = ?1"),
        ("messages", "room_id = ?1"),
        ("room_sequences", "room_id = ?1"),
        ("votes", "room_id = ?1"),
        ("room_tasks", "room_id = ?1"),
        ("subscriptions", "room_id = ?1"),
    ] {
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                [&room.room_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table} must be empty after destruction");
    }
}

#[tokio::test]
async fn test_legacy_ownerless_private_room_fails_closed() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    rusqlite::Connection::open(tmp.path().join("test.db"))
        .unwrap()
        .execute(
            "INSERT INTO rooms (room_id, name, visibility, owner_key)
             VALUES ('legacy-private', 'legacy-private', 'private', NULL)",
            [],
        )
        .unwrap();
    let client = connect_agent(&addr, &key, "reader").await;
    assert!(
        client.room_info("legacy-private").await.is_err(),
        "an authenticated key must not inherit ownerless legacy private rooms"
    );
    assert!(
        client.room_info("lobby").await.is_ok(),
        "the public lobby must remain accessible"
    );
    let rename = client
        .rename_room("legacy-private", "claimed-system-room")
        .await
        .unwrap_err();
    match rename {
        cowchat_client::ClientError::Server { code, .. } => {
            assert_eq!(code, cowchat_core::ErrorCode::AccessDenied)
        }
        other => panic!("expected access_denied, got {other:?}"),
    }
}

#[tokio::test]
async fn test_nonmember_cannot_cast_room_vote() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let owner = connect_agent(&addr, &key, "owner").await;
    let outsider = connect_agent(&addr, &key, "outsider").await;
    let room = owner
        .create_room_with_options("public-vote-room", None, None, true, false)
        .await
        .unwrap();
    owner.join_room(&room.room_id).await.unwrap();
    let vote = owner
        .create_vote(
            &room.room_id,
            "members only",
            None,
            vec!["yes".into(), "no".into()],
            None,
        )
        .await
        .unwrap();

    let result = outsider.cast_vote(&vote.vote_id, 0).await;
    assert!(result.is_err(), "a nonmember must not cast a room vote");
}

// --- Message size cap ---

#[tokio::test]
async fn test_message_size_cap() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let agent = connect_agent(&addr, &key, "bigmouth").await;
    let room = agent.create_room("sizetest", None, None).await.unwrap();
    agent.join_room(&room.room_id).await.unwrap();

    // A normal-sized message passes (free tier cap is 64 KiB).
    agent
        .send_message(&room.room_id, "hello", None, vec![])
        .await
        .expect("a small message must be accepted");

    // One byte over the free-tier cap is rejected with MessageTooLarge.
    let oversized = "x".repeat(64 * 1024 + 1);
    let err = agent
        .send_message(&room.room_id, &oversized, None, vec![])
        .await
        .unwrap_err();
    match err {
        cowchat_client::ClientError::Server { code, .. } => assert_eq!(
            code,
            cowchat_core::ErrorCode::MessageTooLarge,
            "an oversized message must be rejected with MessageTooLarge"
        ),
        other => panic!("expected MessageTooLarge, got {other:?}"),
    }

    // Thinking shares the same cap — it can't be used to dodge the limit.
    let err = agent.thinking(&room.room_id, &oversized).await.unwrap_err();
    match err {
        cowchat_client::ClientError::Server { code, .. } => assert_eq!(
            code,
            cowchat_core::ErrorCode::MessageTooLarge,
            "an oversized thinking pulse must be rejected with MessageTooLarge"
        ),
        other => panic!("expected MessageTooLarge, got {other:?}"),
    }
}

// --- Protocol version gate ---

/// Send one raw register frame and return the server's first reply frame.
async fn raw_register(addr: &str, payload: serde_json::Value) -> Frame {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let frame = serde_json::json!({ "id": "raw-reg", "type": "register", "payload": payload });
    let line = format!("{}\n", serde_json::to_string(&frame).unwrap());
    write_half.write_all(line.as_bytes()).await.unwrap();

    let mut reader = BufReader::new(read_half);
    let mut resp = String::new();
    reader.read_line(&mut resp).await.unwrap();
    serde_json::from_str(resp.trim()).unwrap()
}

#[tokio::test]
async fn test_protocol_version_gate() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    // A client claiming a version newer than the server supports is rejected
    // with a clear UnsupportedProtocol error, not a parse failure.
    let reply = raw_register(
        &addr,
        serde_json::json!({ "key": key, "name": "fromfuture", "protocol_version": 999 }),
    )
    .await;
    assert_eq!(reply.frame_type, FrameType::Error);
    let err: cowchat_core::ErrorPayload = serde_json::from_value(reply.payload).unwrap();
    assert_eq!(
        err.code,
        cowchat_core::ErrorCode::UnsupportedProtocol,
        "a future protocol version must be rejected"
    );

    // A pre-versioning client (no protocol_version field) and an explicit v1
    // client cannot safely consume v2-only RoomUpdated events and fail closed.
    for payload in [
        serde_json::json!({ "key": key, "name": "legacy-absent" }),
        serde_json::json!({ "key": key, "name": "legacy-v1", "protocol_version": 1 }),
    ] {
        let reply = raw_register(&addr, payload).await;
        assert_eq!(reply.frame_type, FrameType::Error);
        let err: cowchat_core::ErrorPayload = serde_json::from_value(reply.payload).unwrap();
        assert_eq!(err.code, cowchat_core::ErrorCode::UnsupportedProtocol);
    }

    let reply = raw_register(
        &addr,
        serde_json::json!({ "key": key, "name": "current-v2", "protocol_version": 2 }),
    )
    .await;
    assert_eq!(reply.frame_type, FrameType::Ok);
    assert_eq!(
        reply
            .payload
            .get("protocol_version")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
}

// --- Room invites ---

/// Like `start_test_server` but with the HTTP listener enabled (signup stays
/// disabled — invites must work independently of `--enable-http-signup`).
async fn start_test_server_with_http() -> (
    tokio::task::JoinHandle<()>,
    String,
    String,
    String,
    tempfile::TempDir,
) {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let key_path = tmp_dir.path().join("auth.key");
    let socket_path = tmp_dir.path().join("test.sock");

    let mut addrs = Vec::new();
    for _ in 0..2 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        addrs.push(listener.local_addr().unwrap().to_string());
        drop(listener);
    }
    let tcp_addr = addrs[0].clone();
    let http_addr = addrs[1].clone();

    let config = ServerConfig {
        socket_path,
        tcp_addr: Some(tcp_addr.clone()),
        http_addr: Some(http_addr.clone()),
        db_path,
        auth_key_path: key_path,
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
    };

    let server = CowchatServer::new(config).unwrap();
    let api_key = server.api_key().to_string();
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    sleep(Duration::from_millis(150)).await;

    (handle, tcp_addr, http_addr, api_key, tmp_dir)
}

async fn redeem_invite_http(http_addr: &str, token: &str) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{http_addr}/api/invites/redeem"))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response
        .json::<serde_json::Value>()
        .await
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn test_single_use_invite_vends_key_and_private_room_access() {
    let (_handle, addr, http_addr, owner_key, _tmp) = start_test_server_with_http().await;

    let owner = connect_agent(&addr, &owner_key, "owner").await;
    let room = owner.create_room("invite-lab", None, None).await.unwrap();
    owner.join_room(&room.room_id).await.unwrap();
    owner
        .send_message(&room.room_id, "welcome aboard", None, vec![])
        .await
        .unwrap();

    let invite = owner.create_invite(&room.room_id, true).await.unwrap();
    assert!(invite.token.starts_with("cinv_"));
    assert_eq!(invite.room_id, room.room_id);
    assert_eq!(invite.room_name, "invite-lab");
    assert!(invite.single_use);

    // Open signup is off; the invite must redeem anyway.
    let signup = reqwest::Client::new()
        .post(format!("http://{http_addr}/api/keys"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(signup.status().as_u16(), 403);

    let (status, body) = redeem_invite_http(&http_addr, &invite.token).await;
    assert_eq!(status, 201);
    assert_eq!(body["room_id"], room.room_id.as_str());
    assert_eq!(body["room_name"], "invite-lab");
    assert_eq!(body["tier"], "free");
    let minted_key = body["api_key"].as_str().unwrap().to_string();
    assert_ne!(minted_key, owner_key);

    // Single-use invites self-destruct: the second redeem fails generically.
    let (reused_status, _) = redeem_invite_http(&http_addr, &invite.token).await;
    assert_eq!(reused_status, 404);

    // The stranger can now see, join, speak in, and read the private room.
    let stranger = connect_agent(&addr, &minted_key, "stranger").await;
    let visible = stranger.list_rooms(None).await.unwrap();
    assert!(
        visible.iter().any(|r| r.room_id == room.room_id),
        "granted private room must appear in the stranger's list_rooms"
    );
    stranger.join_room(&room.room_id).await.unwrap();
    stranger
        .send_message(&room.room_id, "howdy from outside", None, vec![])
        .await
        .unwrap();
    let history = stranger.get_history(&room.room_id, 10, None).await.unwrap();
    let contents: Vec<&str> = history.iter().map(|m| m.content.as_str()).collect();
    assert!(contents.contains(&"welcome aboard"));
    assert!(contents.contains(&"howdy from outside"));

    // Creating an invite requires access: another private room stays closed…
    let sealed = owner
        .create_room("no-invites-here", None, None)
        .await
        .unwrap();
    let denied = stranger.create_invite(&sealed.room_id, true).await;
    assert!(
        matches!(
            denied,
            Err(cowchat_client::ClientError::Server {
                code: cowchat_core::ErrorCode::AccessDenied,
                ..
            })
        ),
        "stranger must not mint invites for rooms it cannot access"
    );
    // …while the granted room is accessible, so minting there is allowed.
    stranger.create_invite(&room.room_id, true).await.unwrap();
}

#[tokio::test]
async fn test_open_invite_redeems_repeatedly_and_revocation_is_owner_scoped() {
    let (_handle, addr, http_addr, owner_key, _tmp) = start_test_server_with_http().await;

    let owner = connect_agent(&addr, &owner_key, "owner").await;
    let room = owner.create_room("open-door", None, None).await.unwrap();
    let invite = owner.create_invite(&room.room_id, false).await.unwrap();
    assert!(!invite.single_use);

    // Open invites redeem repeatedly, minting a distinct key each time.
    let (first_status, first) = redeem_invite_http(&http_addr, &invite.token).await;
    let (second_status, second) = redeem_invite_http(&http_addr, &invite.token).await;
    assert_eq!((first_status, second_status), (201, 201));
    let first_key = first["api_key"].as_str().unwrap().to_string();
    let second_key = second["api_key"].as_str().unwrap().to_string();
    assert_ne!(first_key, second_key);

    // A granted stranger is neither the invite's creator nor the room owner,
    // so it cannot revoke the invite.
    let stranger = connect_agent(&addr, &first_key, "stranger").await;
    let denied = stranger.revoke_invite(&invite.token).await;
    assert!(matches!(
        denied,
        Err(cowchat_client::ClientError::Server {
            code: cowchat_core::ErrorCode::AccessDenied,
            ..
        })
    ));

    // The owner revokes; further redemption fails with the generic 404.
    owner.revoke_invite(&invite.token).await.unwrap();
    let (revoked_status, _) = redeem_invite_http(&http_addr, &invite.token).await;
    assert_eq!(revoked_status, 404);

    // Revoking an unknown token reports InviteNotFound.
    let missing = owner.revoke_invite("cinv_does_not_exist").await;
    assert!(matches!(
        missing,
        Err(cowchat_client::ClientError::Server {
            code: cowchat_core::ErrorCode::InviteNotFound,
            ..
        })
    ));

    // Keys minted before revocation keep their grants.
    for key in [&first_key, &second_key] {
        let holder = connect_agent(&addr, key, "holder").await;
        let visible = holder.list_rooms(None).await.unwrap();
        assert!(visible.iter().any(|r| r.room_id == room.room_id));
        holder.join_room(&room.room_id).await.unwrap();
    }
}

#[tokio::test]
async fn test_list_invites_reports_metadata_and_revoke_by_id() {
    let (_handle, addr, http_addr, owner_key, _tmp) = start_test_server_with_http().await;

    let owner = connect_agent(&addr, &owner_key, "owner").await;
    let room = owner.create_room("invite-desk", None, None).await.unwrap();

    // Listing requires room access: a stranger with its own key sees nothing.
    let single = owner.create_invite(&room.room_id, true).await.unwrap();
    let open = owner.create_invite(&room.room_id, false).await.unwrap();

    let list = owner.list_invites(&room.room_id).await.unwrap();
    assert_eq!(list.room_id, room.room_id);
    assert_eq!(list.invites.len(), 2);
    // Newest first: the open invite was minted second.
    let open_entry = &list.invites[0];
    let single_entry = &list.invites[1];
    assert!(!open_entry.single_use);
    assert!(single_entry.single_use);
    for entry in &list.invites {
        // The invite_id is the SHA-256 hex of the token — opaque, never cinv_.
        assert_eq!(entry.invite_id.len(), 64);
        assert!(entry.invite_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(entry.room_id, room.room_id);
        assert_eq!(entry.redeemed_count, 0);
        assert!(!entry.revoked);
        assert!(!entry.created_at.is_empty());
        assert!(entry.mine, "owner minted both invites");
    }
    assert!(
        !list
            .invites
            .iter()
            .any(|i| i.invite_id == single.token || i.invite_id == open.token),
        "listing must never expose redeemable tokens"
    );

    // A stranger holding a granted key sees the metadata with mine = false.
    let (status, body) = redeem_invite_http(&http_addr, &single.token).await;
    assert_eq!(status, 201);
    let minted_key = body["api_key"].as_str().unwrap().to_string();
    let stranger = connect_agent(&addr, &minted_key, "stranger").await;
    let strangers_view = stranger.list_invites(&room.room_id).await.unwrap();
    assert_eq!(strangers_view.invites.len(), 2);
    assert!(
        strangers_view.invites.iter().all(|i| !i.mine),
        "the stranger minted none of these invites"
    );
    // The single-use invite is now consumed: redeemed once, revoked.
    let consumed = strangers_view
        .invites
        .iter()
        .find(|i| i.single_use)
        .unwrap();
    assert_eq!(consumed.redeemed_count, 1);
    assert!(consumed.revoked);

    // A key without access to the room cannot list at all.
    let sealed = owner.create_room("sealed-desk", None, None).await.unwrap();
    let denied = stranger.list_invites(&sealed.room_id).await;
    assert!(matches!(
        denied,
        Err(cowchat_client::ClientError::Server {
            code: cowchat_core::ErrorCode::AccessDenied,
            ..
        })
    ));

    // The owner revokes the open invite by its listed id; the list reflects it
    // and redemption stops.
    let open_id = list
        .invites
        .iter()
        .find(|i| !i.single_use)
        .unwrap()
        .invite_id
        .clone();
    owner.revoke_invite_by_id(&open_id).await.unwrap();
    let after = owner.list_invites(&room.room_id).await.unwrap();
    let open_after = after.invites.iter().find(|i| !i.single_use).unwrap();
    assert!(open_after.revoked);
    let (revoked_status, _) = redeem_invite_http(&http_addr, &open.token).await;
    assert_eq!(revoked_status, 404);

    // Revoking an unknown id reports InviteNotFound.
    let missing = owner.revoke_invite_by_id(&"0".repeat(64)).await;
    assert!(matches!(
        missing,
        Err(cowchat_client::ClientError::Server {
            code: cowchat_core::ErrorCode::InviteNotFound,
            ..
        })
    ));
}
