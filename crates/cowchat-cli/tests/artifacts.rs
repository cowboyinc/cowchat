use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use serde_json::{json, Value};

#[tokio::test]
async fn artifact_cli_publishes_merges_retries_and_revokes_through_authenticated_http() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = CowchatServer::new(ServerConfig {
        socket_path: dir.path().join("server.sock"),
        tcp_addr: None,
        http_addr: Some(addr.to_string()),
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
        max_rooms: None,
    })
    .unwrap();
    let store = server.store().clone();
    let key = server.api_key().to_owned();
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let url = format!("ws://{addr}/ws");
    let owner = CowchatClient::connect_ws(&url, &key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    owner.create_room("status room", None, None).await.unwrap();
    let room_id = owner
        .list_rooms(None)
        .await
        .unwrap()
        .into_iter()
        .find(|room| room.name == "status room")
        .unwrap()
        .room_id;
    let view = dir.path().join("view.openui");
    let data = dir.path().join("data.json");
    std::fs::write(&view, "root = Stack([Text($summary.text)])").unwrap();
    std::fs::write(
        &data,
        json!({"summary":{"text":"Review"},"items":{"a":{"status":"open"},"b":{"status":"open"}}})
            .to_string(),
    )
    .unwrap();
    let command = |args: Vec<String>| {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"));
        cmd.args([
            "--url",
            &url,
            "--key",
            &key,
            "--name",
            "publisher",
            "--agent-id",
            "publisher",
            "artifact",
        ]);
        cmd.args(args);
        cmd
    };
    let output = command(vec![
        "publish".into(),
        "status room".into(),
        "--title".into(),
        "Status".into(),
        "--view".into(),
        view.to_str().unwrap().into(),
        "--data".into(),
        data.to_str().unwrap().into(),
    ])
    .output()
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let published: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(published["revision"], 1);
    let original_url = published["url"].as_str().unwrap();
    let request_id = uuid::Uuid::new_v4().to_string();
    let update = vec![
        "--request-id".into(),
        request_id,
        "update".into(),
        "status room".into(),
        "--json".into(),
        r#"{"items":{"a":{"status":"done"}}}"#.into(),
    ];
    for _ in 0..2 {
        let output = command(update.clone()).output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let current: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(current["revision"], 2);
        assert_eq!(current["data"]["items"]["a"]["status"], "done");
        assert_eq!(current["data"]["items"]["b"]["status"], "open");
        assert_eq!(current["url"], original_url);
    }
    let stale = command(vec![
        "update".into(),
        "status room".into(),
        "--json".into(),
        "{}".into(),
        "--if-revision".into(),
        "1".into(),
    ])
    .output()
    .await
    .unwrap();
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("409"));
    let response = reqwest::get(format!("{original_url}/snapshot"))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let snapshot: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(snapshot["revision"], 2);
    assert!(snapshot.get("room_id").is_none());
    let output = command(vec!["rotate-link".into(), "status room".into()])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rotated: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(rotated["url"], original_url);
    assert_eq!(
        reqwest::get(format!("{original_url}/snapshot"))
            .await
            .unwrap()
            .status(),
        404
    );
    let output = command(vec!["unpublish".into(), "status room".into()])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        reqwest::get(format!("{}/snapshot", rotated["url"].as_str().unwrap()))
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(
        !store
            .room_voters(&room_id)
            .unwrap()
            .iter()
            .any(|agent| agent == "publisher"),
        "An artifact-only publisher must not become a room voter"
    );
    task.abort();
}
