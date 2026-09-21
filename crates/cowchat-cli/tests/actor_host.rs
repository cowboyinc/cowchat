//! Runs the real CLI receiver and a cold subprocess against encrypted room traffic.
use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

async fn start_host(addr: &str, key: &str, room: &str, port: &str) -> tokio::process::Child {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/python/fizzbuzz_actor.py");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            addr,
            "--key",
            key,
            "--name",
            "FizzBuzz",
            "--agent-id",
            "fizzbuzz-actor",
            "actor-host",
            room,
            "--listen",
            port,
            "--",
            "python3",
        ])
        .arg(fixture)
        .env("COWCHAT_ROOM_KEY", "local-proof-secret")
        .env("COWCHAT_WAKE_SECRET", "local-wake-secret")
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

async fn reply(client: &CowchatClient, room: &str, input_id: &str) -> String {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            for message in client.get_history(room, 100, None).await.unwrap() {
                if message.reply_to_message.as_deref() == Some(input_id) {
                    return message.content;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn actor_host_fizzbuzz_recovers_pending_work_after_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap().to_string();
    drop(tcp);
    let wake = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = wake.local_addr().unwrap().to_string();
    drop(wake);
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
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut human = CowchatClient::connect_tcp(&addr, &key, "Human", Some("human"), vec![])
        .await
        .unwrap();
    let mut observer =
        CowchatClient::connect_tcp(&addr, &key, "Observer", Some("observer"), vec![])
            .await
            .unwrap();
    human.set_room_secret(b"local-proof-secret");
    observer.set_room_secret(b"local-proof-secret");
    let room = human
        .create_room_with_options("fizzbuzz-proof", None, None, false, true)
        .await
        .unwrap();
    human.join_room(&room.room_id).await.unwrap();
    observer.join_room(&room.room_id).await.unwrap();
    let mut host = start_host(&addr, &key, &room.room_id, &port).await;
    let first = human
        .send_message(&room.room_id, "15", None, vec!["fizzbuzz-actor".into()])
        .await
        .unwrap();
    assert_eq!(
        reply(&observer, &room.room_id, &first.message_id)
            .await
            .trim(),
        "FizzBuzz"
    );
    let poison = human
        .send_message(
            &room.room_id,
            "not an integer",
            None,
            vec!["fizzbuzz-actor".into()],
        )
        .await
        .unwrap();
    let after_poison = human
        .send_message(&room.room_id, "5", None, vec!["fizzbuzz-actor".into()])
        .await
        .unwrap();
    assert_eq!(
        reply(&observer, &room.room_id, &after_poison.message_id)
            .await
            .trim(),
        "Buzz"
    );
    assert!(!observer
        .get_history(&room.room_id, 100, None)
        .await
        .unwrap()
        .iter()
        .any(|m| m.reply_to_message.as_deref() == Some(poison.message_id.as_str())));
    host.kill().await.unwrap(); // SIGKILL on Unix: no graceful actor-host cleanup
    host.wait().await.unwrap();
    let pending = human
        .send_message(&room.room_id, "3", None, vec!["fizzbuzz-actor".into()])
        .await
        .unwrap();
    // No actor receiver is running. The committed obligation survives independently.
    let mut restarted = start_host(&addr, &key, &room.room_id, &port).await;
    assert_eq!(
        reply(&observer, &room.room_id, &pending.message_id)
            .await
            .trim(),
        "Fizz"
    );
    let history = observer
        .get_history(&room.room_id, 100, None)
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|m| m.reply_to_message.as_deref() == Some(first.message_id.as_str()))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|m| m.reply_to_message.as_deref() == Some(pending.message_id.as_str()))
            .count(),
        1
    );
    restarted.kill().await.unwrap();
    restarted.wait().await.unwrap();
    task.abort();
}
