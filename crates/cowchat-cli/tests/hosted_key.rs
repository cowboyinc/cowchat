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
        db_path: db_path.clone(),
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
    let initial = store.get_history(room, 10, None).unwrap();
    assert_eq!(initial.last().unwrap().metadata["display_external"], false);
    let visible = bound_command(&[
        "--agent-id",
        &id,
        "send",
        room,
        "human update",
        "--display-external",
        "true",
    ])
    .await
    .unwrap();
    assert!(visible.status.success());
    let with_visible = store.get_history(room, 10, None).unwrap();
    assert_eq!(
        with_visible.last().unwrap().metadata["display_external"],
        true
    );
    let hidden = bound_command(&[
        "--agent-id",
        &id,
        "send",
        room,
        "private update",
        "--display-external",
        "false",
    ])
    .await
    .unwrap();
    assert!(hidden.status.success());
    let with_hidden = store.get_history(room, 10, None).unwrap();
    assert_eq!(
        with_hidden.last().unwrap().metadata["display_external"],
        false
    );
    let thought = bound_command(&["--agent-id", &id, "thinking", room, "investigating"])
        .await
        .unwrap();
    assert!(thought.status.success());
    let with_thought = store.get_history(room, 10, None).unwrap();
    assert_eq!(
        with_thought.last().unwrap().metadata["display_external"],
        false
    );
    let offline_prompt = bound_command(&[
        "--agent-id",
        &id,
        "presence",
        "prompt",
        "--detail",
        "permission requested",
    ])
    .await
    .unwrap();
    assert!(
        offline_prompt.status.success(),
        "{}",
        String::from_utf8_lossy(&offline_prompt.stderr)
    );
    assert_eq!(store.get_presence(&id).unwrap().unwrap().0, "prompt");
    let oversized_detail = "x".repeat(257);
    let oversized = bound_command(&[
        "--agent-id",
        &id,
        "presence",
        "working",
        "--detail",
        &oversized_detail,
    ])
    .await
    .unwrap();
    assert!(!oversized.status.success());
    assert_eq!(store.get_presence(&id).unwrap().unwrap().0, "prompt");
    let pull_request = dir.path().join("pull-request.json");
    std::fs::write(
        &pull_request,
        serde_json::json!({
            "repository":"cowboyinc/cowchat", "number":90, "title":"Finish work",
            "checks":"passing", "additions":12, "deletions":3,
            "address":"https://github.com/cowboyinc/cowchat/pull/90"
        })
        .to_string(),
    )
    .unwrap();
    let screenshot_a = dir.path().join("first.png");
    let screenshot_b = dir.path().join("second.png");
    std::fs::write(&screenshot_a, b"first screenshot").unwrap();
    std::fs::write(&screenshot_b, b"second screenshot").unwrap();
    let finished = bound_command(&[
        "--agent-id",
        &id,
        "send",
        room,
        "ready for review",
        "--pull-request",
        pull_request.to_str().unwrap(),
        "--attach",
        screenshot_a.to_str().unwrap(),
        "--attach",
        screenshot_b.to_str().unwrap(),
    ])
    .await
    .unwrap();
    assert!(
        finished.status.success(),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
    let history = store.get_history(room, 10, None).unwrap();
    let finished_message = history.last().unwrap();
    assert_eq!(finished_message.metadata["pull_request"]["number"], 90);
    let attachments = finished_message.metadata["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);
    let blob_ids = attachments
        .iter()
        .map(|file| file["blob_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    for blob_id in &blob_ids {
        assert!(store.get_blob(blob_id).unwrap().is_some());
    }
    let shared = store
        .social_action(
            owner["device_key"].as_str().unwrap(),
            "POST",
            "rooms",
            &serde_json::json!({"name":"shared-work"}),
        )
        .unwrap();
    let shared_id = shared["room_id"].as_str().unwrap();
    let cursor = dir.path().join("approved-rooms-cursor");
    let mut waiter = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--url",
            &url,
            "--name",
            "worker",
            "--agent-id",
            &id,
            "wait",
            room,
            "--approved-rooms",
            "--mentions-only",
            "--loop",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--since-seq",
            "0",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "1",
        ])
        .env("HOME", &home)
        .env("COWCHAT_SESSION_FILE", session_file)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE social_sessions SET last_seen=unixepoch()-200 WHERE id=?1",
        [&id],
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let last_seen: i64 = conn
        .query_row(
            "SELECT last_seen FROM social_sessions WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(last_seen > chrono::Utc::now().timestamp() - 10);
    let sent_while_waiting = bound_command(&["--agent-id", &id, "send", room, "still working"])
        .await
        .unwrap();
    assert!(
        sent_while_waiting.status.success(),
        "{}",
        String::from_utf8_lossy(&sent_while_waiting.stderr)
    );
    let sent_history = store.get_history(room, 10, None).unwrap();
    let sent_message = sent_history
        .iter()
        .find(|message| message.content == "still working")
        .unwrap();
    assert!(sent_message.agent_name.starts_with("worker · @"));
    let presence_while_waiting = bound_command(&[
        "--agent-id",
        &id,
        "presence",
        "working",
        "--detail",
        "reviewing",
    ])
    .await
    .unwrap();
    assert!(
        presence_while_waiting.status.success(),
        "{}",
        String::from_utf8_lossy(&presence_while_waiting.stderr)
    );
    assert!(
        waiter.try_wait().unwrap().is_none(),
        "send and presence must not end the waiter"
    );
    let approval = reqwest::Client::new()
        .post(format!("http://{addr}/social/v1/rooms/{shared_id}/agents"))
        .header("x-cowchat-key", owner["device_key"].as_str().unwrap())
        .json(&serde_json::json!({"agent_ids":[id]}))
        .send()
        .await
        .unwrap();
    assert!(approval.status().is_success());
    let human = CowchatClient::connect_ws(
        &url,
        owner["device_key"].as_str().unwrap(),
        "owner",
        None,
        vec![],
    )
    .await
    .unwrap();
    human.join_room(shared_id).await.unwrap();
    human
        .send_message(shared_id, "please review this", None, vec![id.clone()])
        .await
        .unwrap();
    let waiter_out =
        tokio::time::timeout(std::time::Duration::from_secs(6), waiter.wait_with_output())
            .await
            .unwrap()
            .unwrap();
    assert!(
        waiter_out.status.success(),
        "{}",
        String::from_utf8_lossy(&waiter_out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&waiter_out.stdout).contains("please review this"),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&waiter_out.stdout),
        String::from_utf8_lossy(&waiter_out.stderr)
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
    assert!(
        String::from_utf8_lossy(&insecure.stderr).contains("expected ws or wss URL"),
        "{}",
        String::from_utf8_lossy(&insecure.stderr)
    );
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
    assert!(store.get_presence(&id).unwrap().is_none());
    assert!(!store.room_presence_allowed(room, &id));
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
    let claude_id = uuid::Uuid::new_v4().to_string();
    let hook = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/cowchat-claude/hooks/session.py");
    let env_file = dir.path().join("claude-env");
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_cowchat"));
    let path = format!(
        "{}:{}",
        executable.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run_hook = || {
        let mut process = tokio::process::Command::new("python3");
        process
            .arg(&hook)
            .arg("start")
            .env("HOME", &home)
            .env("PATH", &path)
            .env("CLAUDE_ENV_FILE", &env_file)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut first_hook = run_hook();
    use tokio::io::AsyncWriteExt;
    first_hook
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({"session_id":claude_id,"cwd":home})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let first_hook = first_hook.wait_with_output().await.unwrap();
    assert!(first_hook.status.success());
    let env_text = std::fs::read_to_string(&env_file).unwrap();
    let first_id = env_text
        .lines()
        .find_map(|line| line.strip_prefix("export COWCHAT_AGENT_ID="))
        .unwrap()
        .to_string();
    conn.execute(
        "UPDATE social_sessions SET last_seen=unixepoch()-301 WHERE id=?1",
        [&first_id],
    )
    .unwrap();
    store.social_expire_sessions().unwrap();
    let mut resumed_hook = run_hook();
    resumed_hook
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({"session_id":claude_id,"cwd":home})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let resumed_hook = resumed_hook.wait_with_output().await.unwrap();
    assert!(
        resumed_hook.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed_hook.stderr)
    );
    let env_text = std::fs::read_to_string(&env_file).unwrap();
    let ids = env_text
        .lines()
        .filter_map(|line| line.strip_prefix("export COWCHAT_AGENT_ID="))
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    task.abort();
    std::env::remove_var("COWCHAT_SOCIAL_PUBLIC_ORIGIN");
}
