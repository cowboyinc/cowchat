use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn send_uses_url_bound_key_without_a_flag_or_local_key_file() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = CowchatServer::new(ServerConfig {
        socket_path: dir.path().join("server.sock"),
        tcp_addr: None,
        http_addr: Some(addr.to_string()),
        db_path: dir.path().join("server.db"),
        auth_key_path: dir.path().join("server-auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: false,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
        max_rooms: None,
    })
    .unwrap();
    let key = server.api_key().to_owned();
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let url = format!("ws://{addr}/ws");
    let owner = CowchatClient::connect_ws(&url, &key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room = owner
        .create_room("hosted-key-test", None, None)
        .await
        .unwrap();
    let home = dir.path().join("home");
    let cowchat = home.join(".cowchat");
    std::fs::create_dir_all(&cowchat).unwrap();
    let key_file = cowchat.join("hosted-bot.json");
    std::fs::write(
        &key_file,
        serde_json::json!({"url":url,"api_key":key}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!cowchat.join("auth.key").exists());

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--url",
            &url,
            "--name",
            "sender",
            "--agent-id",
            "sender",
            "send",
            "hosted-key-test",
            "hello from saved key",
        ])
        .env("HOME", &home)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let history = owner.get_history(&room.room_id, 20, None).await.unwrap();
    assert!(history
        .iter()
        .any(|message| message.content == "hello from saved key"));
    task.abort();
}
