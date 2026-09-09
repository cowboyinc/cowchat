use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;

async fn start_test_server() -> (
    tokio::task::JoinHandle<()>,
    String,
    String,
    tempfile::TempDir,
) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let config = ServerConfig {
        socket_path: temp_dir.path().join("test.sock"),
        tcp_addr: Some(tcp_addr.clone()),
        http_addr: None,
        db_path: temp_dir.path().join("test.db"),
        auth_key_path: temp_dir.path().join("auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: false,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
    };
    let server = CowchatServer::new(config).unwrap();
    let api_key = server.api_key().to_string();
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    sleep(Duration::from_millis(100)).await;

    (handle, tcp_addr, api_key, temp_dir)
}

async fn rooms_list(addr: &str, key: &str, extra_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["--tcp", addr, "--key", key, "rooms", "list"])
        .args(extra_args)
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn room_list_json_reports_described_room_and_empty_parent() {
    let (_server, addr, key, _temp_dir) = start_test_server().await;
    let creator = CowchatClient::connect_tcp(
        &addr,
        &key,
        "room-test-creator",
        Some("room-test-creator"),
        vec![],
    )
    .await
    .unwrap();
    let room = creator
        .create_room("review", Some("Review PR 42"), None)
        .await
        .unwrap();

    let output = rooms_list(&addr, &key, &["--json"]).await;
    assert!(
        output.status.success(),
        "rooms list --json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rooms = parsed["rooms"].as_array().unwrap();
    let listed = rooms
        .iter()
        .find(|candidate| candidate["room_id"] == room.room_id)
        .expect("created room should be listed");
    assert_eq!(listed["name"], "review");
    assert_eq!(listed["description"], "Review PR 42");

    let output = rooms_list(&addr, &key, &["--parent", room.room_id.as_str(), "--json"]).await;
    assert!(
        output.status.success(),
        "empty parent query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"rooms": []})
    );
}
