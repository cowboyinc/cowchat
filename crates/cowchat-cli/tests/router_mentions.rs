use base64::Engine;
use cowchat_server::{CowchatServer, ServerConfig};
use sha2::Digest;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cowchat");

struct Session {
    id: String,
    file: String,
}

async fn start_session(url: &str, home: &Path, name: &str) -> Session {
    let id = format!("session:{}", uuid::Uuid::new_v4());
    let output = Command::new(BIN)
        .args([
            "--url", url, "--name", name, "session", "start", "--id", &id,
        ])
        .args(["--harness", "codex", "--folder", name])
        .env("HOME", home)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let started: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    Session {
        id,
        file: started["session_file"].as_str().unwrap().to_owned(),
    }
}

fn as_session(url: &str, home: &Path, session: &Session, args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command
        .args(["--url", url, "--agent-id", &session.id])
        .args(args)
        .env("HOME", home)
        .env("COWCHAT_SESSION_FILE", &session.file);
    command
}

fn server_config(dir: &Path, addr: &std::net::SocketAddr, db_path: PathBuf) -> ServerConfig {
    ServerConfig {
        socket_path: dir.join("server.sock"),
        tcp_addr: None,
        http_addr: Some(addr.to_string()),
        db_path,
        auth_key_path: dir.join("server-auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: false,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
        max_rooms: None,
    }
}

#[tokio::test]
async fn a_routed_session_wakes_on_the_routers_follow_up() {
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
        .social_oauth_complete("github", &state, "router-owner", "Owner")
        .unwrap();
    let owner = store
        .social_oauth_exchange(&request, &verifier, &code)
        .unwrap();
    let device_key = owner["device_key"].as_str().unwrap();
    let bot = store
        .social_action(
            device_key,
            "POST",
            "agents",
            &serde_json::json!({"name":"Cowbot","mint_only":true}),
        )
        .unwrap();
    let server = CowchatServer::new(server_config(dir.path(), &addr, db_path)).unwrap();
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let url = format!("ws://{addr}/ws");
    let home = dir.path().join("home");
    let cowchat = home.join(".cowchat");
    std::fs::create_dir_all(&cowchat).unwrap();
    let hosted = cowchat.join("hosted-bot.json");
    std::fs::write(
        &hosted,
        serde_json::json!({"url":url,"api_key":bot["api_key"],"kind":"mint_only"}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&hosted, std::fs::Permissions::from_mode(0o600)).unwrap();

    let router = start_session(&url, &home, "router").await;
    let worker = start_session(&url, &home, "worker").await;
    let author = start_session(&url, &home, "author").await;
    let room = store
        .social_action(
            device_key,
            "POST",
            "rooms",
            &serde_json::json!({"name":"shared-work"}),
        )
        .unwrap();
    let room_id = room["room_id"].as_str().unwrap();
    store
        .social_action(
            device_key,
            "POST",
            &format!("rooms/{room_id}/agents"),
            &serde_json::json!({"agent_ids":[router.id, worker.id, author.id]}),
        )
        .unwrap();

    let sent = as_session(&url, &home, &author, &["send", room_id, "core#1 is ready"])
        .output()
        .await
        .unwrap();
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    let original = store.get_history(room_id, 10, None).unwrap();
    let original = original.last().unwrap().clone();

    // The author's own route must not wake it, and a route to a session that is
    // not in the room must not cost the worker its wake.
    let routes = dir.path().join("routes.tsv");
    std::fs::write(
        &routes,
        format!(
            "{}\tAuthor\tcore#1\\b\t\t\n{}\tWorker\tcore#1\\b\t\t\nsession:gone\tGone\tcore#1\\b\t\t\n",
            author.id, worker.id
        ),
    )
    .unwrap();
    let posted = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "'{BIN}' --url '{url}' --agent-id '{id}' wait '{room_id}' --since-seq 0 --drain --timeout 5 \
             | '{BIN}' router classify --routes '{routes}' --no-judge \
             | '{BIN}' --url '{url}' --agent-id '{id}' router mention '{room_id}'",
            id = router.id,
            routes = routes.display(),
        ))
        .env("HOME", &home)
        .env("COWCHAT_SESSION_FILE", &router.file)
        .output()
        .await
        .unwrap();
    assert!(
        posted.status.success(),
        "{}",
        String::from_utf8_lossy(&posted.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&posted.stdout).unwrap();
    assert_eq!(result["seq"], original.seq);
    assert_eq!(result["mentioned"], serde_json::json!([worker.id]));
    assert_eq!(result["failed"], serde_json::json!(["session:gone"]));
    assert_eq!(result["limited"], serde_json::json!([]));

    let woken = as_session(
        &url,
        &home,
        &worker,
        &[
            "wait",
            room_id,
            "--mentions-only",
            "--since-seq",
            "0",
            "--timeout",
            "5",
        ],
    )
    .output()
    .await
    .unwrap();
    assert!(
        woken.status.success(),
        "{}",
        String::from_utf8_lossy(&woken.stderr)
    );
    let history = store.get_history(room_id, 10, None).unwrap();
    let follow_up = history.last().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(follow_up.metadata["kind"], "routed");
    assert_eq!(
        follow_up.metadata["reasons"],
        serde_json::json!([{"route":worker.id,"rule":"exact"}])
    );
    assert_eq!(
        follow_up.metadata["mentions"],
        serde_json::json!([worker.id])
    );
    assert_eq!(
        follow_up.reply_to_message.as_deref(),
        Some(original.message_id.as_str())
    );
    assert!(String::from_utf8_lossy(&woken.stdout).contains(&format!("#{}", original.seq)));
    task.abort();
}
