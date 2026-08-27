use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::tempdir;
use tokio::process::Command as TokioCommand;
use tokio::time::sleep;

fn run(workdir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .current_dir(workdir)
        .args(args)
        .output()
        .unwrap()
}

async fn start_test_server() -> (
    tokio::task::JoinHandle<()>,
    PathBuf,
    String,
    tempfile::TempDir,
) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let socket_path = temp_dir.path().join("test.sock");

    let config = ServerConfig {
        socket_path: socket_path.clone(),
        tcp_addr: None,
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

    (handle, socket_path, api_key, temp_dir)
}

async fn run_connected(
    workdir: &std::path::Path,
    socket: &Path,
    key: &str,
    args: &[&str],
) -> std::process::Output {
    TokioCommand::new(env!("CARGO_BIN_EXE_cowchat"))
        .current_dir(workdir)
        .args(["--socket", socket.to_str().unwrap(), "--key", key])
        .args(args)
        .output()
        .await
        .unwrap()
}

#[test]
fn workflow_init_creates_discoverable_channel_cards_without_overwriting() {
    let temp = tempdir().unwrap();

    let initialized = run(temp.path(), &["workflow", "init", "software-delivery"]);
    assert!(
        initialized.status.success(),
        "workflow init failed: {}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    assert!(temp.path().join(".cowchat/workflow.toml").is_file());

    let channels = run(temp.path(), &["workflow", "channels", "--json"]);
    assert!(
        channels.status.success(),
        "workflow channels failed: {}",
        String::from_utf8_lossy(&channels.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&channels.stdout).unwrap();
    assert_eq!(parsed["workflow"]["name"], "software-delivery");
    assert!(parsed["channels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|channel| channel["id"] == "handoffs"));

    let repeated = run(temp.path(), &["workflow", "init", "software-delivery"]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("refusing to overwrite"));
    assert!(
        fs::read_to_string(temp.path().join(".cowchat/workflow.toml"))
            .unwrap()
            .contains("[channels.review]")
    );
}

#[tokio::test]
async fn workflow_sync_creates_missing_template_rooms_and_preserves_them_on_retry() {
    let temp = tempdir().unwrap();
    let initialized = run(temp.path(), &["workflow", "init", "software-delivery"]);
    assert!(initialized.status.success());
    let (_server, socket, key, _server_data) = start_test_server().await;
    let creator = CowchatClient::connect_uds(
        &socket,
        &key,
        "workflow-user",
        Some("workflow-user"),
        vec![],
    )
    .await
    .unwrap();
    let existing_review = creator
        .create_room("review", Some("User-owned review room"), None)
        .await
        .unwrap();

    let first = run_connected(temp.path(), &socket, &key, &["workflow", "sync", "--json"]).await;
    assert!(
        first.status.success(),
        "first workflow sync failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["channels"].as_array().unwrap().len(), 4);
    let review = first["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["id"] == "review")
        .unwrap();
    assert_eq!(review["action"], "existing");
    assert_eq!(review["room_id"], existing_review.room_id);
    assert_eq!(
        first["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|channel| channel["action"] == "created")
            .count(),
        3
    );
    assert_eq!(
        creator
            .list_rooms(None)
            .await
            .unwrap()
            .iter()
            .find(|room| room.room_id == existing_review.room_id)
            .unwrap()
            .description
            .as_deref(),
        Some("User-owned review room")
    );

    let second = run_connected(temp.path(), &socket, &key, &["workflow", "sync", "--json"]).await;
    assert!(second.status.success());
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert!(second["channels"]
        .as_array()
        .unwrap()
        .iter()
        .all(|channel| channel["action"] == "existing"));
}
