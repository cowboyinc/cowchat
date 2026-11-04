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

#[tokio::test]
async fn session_start_rejects_wrong_server_metadata_and_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let session_id = format!("session:{}", uuid::Uuid::new_v4());
    let suffix = session_id.strip_prefix("session:").unwrap();
    let sessions = home.join(".cowchat/sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let record = serde_json::json!({
        "url":"wss://chat.example/ws", "id":session_id, "name":"worker",
        "harness":"codex", "folder":"repo", "branch":"main",
        "room_id":"room-1", "room_name":"repo", "session_token":"csoc_token"
    });
    let path = sessions.join(format!("{suffix}.json"));
    std::fs::write(&path, record.to_string()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let run = |url: &'static str, folder: &'static str| {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
            .args([
                "--url",
                url,
                "--name",
                "worker",
                "session",
                "start",
                "--id",
                &session_id,
                "--harness",
                "codex",
                "--folder",
                folder,
                "--branch",
                "main",
            ])
            .env("HOME", home)
            .env("COWCHAT_SESSION_FILE", &path)
            .output()
    };
    assert!(!run("wss://other.example/ws", "repo")
        .await
        .unwrap()
        .status
        .success());
    assert!(!run("wss://chat.example/ws", "other")
        .await
        .unwrap()
        .status
        .success());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!run("wss://chat.example/ws", "repo")
        .await
        .unwrap()
        .status
        .success());
}

#[tokio::test]
async fn saved_mint_key_starts_session_and_cli_uses_only_its_token() {
    use base64::Engine;
    use sha2::Digest;

    std::env::set_var("COWCHAT_SOCIAL_PUBLIC_ORIGIN", "https://chat.example");
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let db_path = dir.path().join("server.db");
    let store = cowchat_server::store::Store::open(&db_path).unwrap();
    let verifier = "v".repeat(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let (request, state) = store
        .social_oauth_start("github", &challenge, None, 49233, "test-source")
        .unwrap();
    store.social_oauth_claim("github", &state).unwrap();
    let (_, code) = store
        .social_oauth_complete("github", &state, "cli-owner", "Owner")
        .unwrap();
    let owner = store
        .social_oauth_exchange(&request, &verifier, &code)
        .unwrap();
    let bot = store
        .social_action(
            owner["device_key"].as_str().unwrap(),
            "POST",
            "agents",
            &serde_json::json!({"name":"Cowbot","mint_only":true}),
        )
        .unwrap();
    let mint_key = bot["api_key"].as_str().unwrap();
    let server = CowchatServer::new(ServerConfig {
        socket_path: dir.path().join("server.sock"),
        tcp_addr: None,
        http_addr: Some(addr.to_string()),
        db_path,
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
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let url = format!("ws://{addr}/ws");
    let home = dir.path().join("home");
    let cowchat = home.join(".cowchat");
    std::fs::create_dir_all(&cowchat).unwrap();
    let hosted = cowchat.join("hosted-bot.json");
    std::fs::write(
        &hosted,
        serde_json::json!({"url":url,"api_key":mint_key,"kind":"mint_only"}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&hosted, std::fs::Permissions::from_mode(0o600)).unwrap();
    let id = format!("session:{}", uuid::Uuid::new_v4());
    let command = |args: &[&str]| {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
            .args(["--url", &url])
            .args(args)
            .env("HOME", &home)
            .output()
    };
    let started = command(&[
        "--name",
        "worker",
        "session",
        "start",
        "--id",
        &id,
        "--harness",
        "codex",
        "--folder",
        "repo",
    ])
    .await
    .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let started: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
    let room = started["room_id"].as_str().unwrap();
    let session_file = started["session_file"].as_str().unwrap();
    assert!(!session_file.ends_with(&format!("{}.json", id.trim_start_matches("session:"))));
    let bound_command = |args: &[&str]| {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
            .args(["--url", &url])
            .args(args)
            .env("HOME", &home)
            .env("COWCHAT_SESSION_FILE", session_file)
            .output()
    };
    let direct = command(&[
        "--name",
        "worker",
        "--agent-id",
        "bot-id",
        "send",
        room,
        "wrong identity",
    ])
    .await
    .unwrap();
    assert!(!direct.status.success());
    let resumed = bound_command(&[
        "--name",
        "worker",
        "session",
        "start",
        "--id",
        &id,
        "--harness",
        "codex",
        "--folder",
        "repo",
    ])
    .await
    .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let sent = bound_command(&["--agent-id", &id, "send", room, "hello from session"])
        .await
        .unwrap();
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    let insecure = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--url",
            "ws://chat.example/ws",
            "--agent-id",
            &id,
            "send",
            room,
            "leak",
        ])
        .env("HOME", &home)
        .env("COWCHAT_SESSION_FILE", session_file)
        .output()
        .await
        .unwrap();
    assert!(!insecure.status.success());
    assert!(String::from_utf8_lossy(&insecure.stderr).contains("expected ws or wss URL"));
    let sibling_id = format!("session:{}", uuid::Uuid::new_v4());
    let sibling = command(&[
        "--name",
        "sibling",
        "session",
        "start",
        "--id",
        &sibling_id,
        "--harness",
        "codex",
        "--folder",
        "other-repo",
    ])
    .await
    .unwrap();
    assert!(
        sibling.status.success(),
        "{}",
        String::from_utf8_lossy(&sibling.stderr)
    );
    let sibling: serde_json::Value = serde_json::from_slice(&sibling.stdout).unwrap();
    let sibling_room = sibling["room_id"].as_str().unwrap();
    let sibling_file = sibling["session_file"].as_str().unwrap();
    assert!(
        !bound_command(&["--agent-id", &sibling_id, "send", sibling_room, "spoofed"])
            .await
            .unwrap()
            .status
            .success()
    );
    assert!(!bound_command(&["session", "end", &sibling_id])
        .await
        .unwrap()
        .status
        .success());
    assert!(!command(&["--agent-id", &id, "send", room, "unbound"])
        .await
        .unwrap()
        .status
        .success());
    let beat = bound_command(&["session", "heartbeat", &id]).await.unwrap();
    assert!(
        beat.status.success(),
        "{}",
        String::from_utf8_lossy(&beat.stderr)
    );
    let ended = bound_command(&["session", "end", &id]).await.unwrap();
    assert!(
        ended.status.success(),
        "{}",
        String::from_utf8_lossy(&ended.stderr)
    );
    assert!(!cowchat
        .join("sessions")
        .join(format!("{}.json", id.trim_start_matches("session:")))
        .exists());
    let sibling_end = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["--url", &url, "session", "end", &sibling_id])
        .env("HOME", &home)
        .env("COWCHAT_SESSION_FILE", sibling_file)
        .output()
        .await
        .unwrap();
    assert!(sibling_end.status.success());
    task.abort();
    std::env::remove_var("COWCHAT_SOCIAL_PUBLIC_ORIGIN");
}
