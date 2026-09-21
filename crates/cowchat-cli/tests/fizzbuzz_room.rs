//! The three-seat FizzBuzz room: counter, fizz, and buzz actor hosts share one
//! encrypted room; mentions route work; replies never cascade into more work.
use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

const ROOM_KEY: &str = "fizzbuzz-room-secret";

async fn start_seat(addr: &str, key: &str, room: &str, role: &str) -> tokio::process::Child {
    let wake = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = wake.local_addr().unwrap().to_string();
    drop(wake);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/python/seat_actor.py");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            addr,
            "--key",
            key,
            "--name",
            role,
            "--agent-id",
            role,
            "actor-host",
            room,
            "--listen",
            &port,
            "--",
            "python3",
        ])
        .arg(fixture)
        .arg(role)
        .env("COWCHAT_ROOM_KEY", ROOM_KEY)
        .env("COWCHAT_WAKE_SECRET", format!("wake-{role}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["status"],
        "ready"
    );
    child
}

/// All replies to one input, as (author, decrypted text), oldest first.
async fn replies(
    client: &CowchatClient,
    room: &str,
    input_id: &str,
    expected: usize,
) -> Vec<(String, String)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let found: Vec<(String, String)> = client
                .get_history(room, 200, None)
                .await
                .unwrap()
                .into_iter()
                .filter(|m| m.reply_to_message.as_deref() == Some(input_id))
                .map(|m| (m.agent_id, m.content.trim().to_owned()))
                .collect();
            if found.len() >= expected {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected {expected} replies to {input_id}"))
}

#[tokio::test]
async fn three_seat_fizzbuzz_room_routes_by_mention_without_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap().to_string();
    drop(tcp);
    let config = ServerConfig {
        socket_path: dir.path().join("server.sock"),
        tcp_addr: Some(addr.clone()),
        http_addr: None,
        db_path: dir.path().join("server.db"),
        auth_key_path: dir.path().join("auth.key"),
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
    let key = server.api_key().to_owned();
    tokio::spawn(async move { server.run().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut chad = CowchatClient::connect_tcp(&addr, &key, "Chad", Some("chad"), vec![])
        .await
        .unwrap();
    chad.set_room_secret(ROOM_KEY.as_bytes());
    let room = chad
        .create_room_with_options("fizzbuzz", None, None, false, true)
        .await
        .unwrap();
    chad.join_room(&room.room_id).await.unwrap();

    let _counter = start_seat(&addr, &key, &room.room_id, "counter").await;
    let mut fizz = start_seat(&addr, &key, &room.room_id, "fizz").await;
    let _buzz = start_seat(&addr, &key, &room.room_id, "buzz").await;

    // The old demo's exact exchanges, now on the v2 stack.
    for (text, seat, want) in [
        ("38", "counter", "39"),
        ("3", "fizz", "Fizz"),
        ("5", "buzz", "Buzz"),
    ] {
        let sent = chad
            .send_message(&room.room_id, text, None, vec![seat.to_owned()])
            .await
            .unwrap();
        let got = replies(&chad, &room.room_id, &sent.message_id, 1).await;
        assert_eq!(
            got,
            vec![(seat.to_owned(), want.to_owned())],
            "input {text}"
        );
    }

    // One message mentioning two seats produces exactly two independent replies.
    let both = chad
        .send_message(
            &room.room_id,
            "15",
            None,
            vec!["fizz".into(), "buzz".into()],
        )
        .await
        .unwrap();
    let mut got = replies(&chad, &room.room_id, &both.message_id, 2).await;
    got.sort();
    assert_eq!(
        got,
        vec![
            ("buzz".to_owned(), "Buzz".to_owned()),
            ("fizz".to_owned(), "Fizz".to_owned()),
        ]
    );
    // No cascade: give any stray wake time to land, then re-count.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        replies(&chad, &room.room_id, &both.message_id, 2)
            .await
            .len(),
        2,
        "actor replies must not trigger further actor work"
    );

    // Kill one seat; the others keep serving; the pending work survives restart.
    fizz.kill().await.unwrap();
    fizz.wait().await.unwrap();
    let while_down = chad
        .send_message(&room.room_id, "9", None, vec!["fizz".into()])
        .await
        .unwrap();
    let counter_live = chad
        .send_message(&room.room_id, "41", None, vec!["counter".into()])
        .await
        .unwrap();
    assert_eq!(
        replies(&chad, &room.room_id, &counter_live.message_id, 1).await[0].1,
        "42"
    );
    let _fizz = start_seat(&addr, &key, &room.room_id, "fizz").await;
    assert_eq!(
        replies(&chad, &room.room_id, &while_down.message_id, 1).await[0].1,
        "Fizz"
    );
}
