use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::sleep;

async fn start_test_server() -> (tokio::task::JoinHandle<()>, PathBuf, String, TempDir) {
    let temp_dir = TempDir::new().unwrap();
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

async fn run(socket: &Path, key: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["--socket", socket.to_str().unwrap(), "--key", key])
        .args(args)
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn handoff_send_list_and_accept_preserve_structured_context() {
    let (_server, socket, key, _server_data) = start_test_server().await;
    let creator = CowchatClient::connect_uds(
        &socket,
        &key,
        "handoff-creator",
        Some("handoff-creator"),
        vec![],
    )
    .await
    .unwrap();
    let room = creator
        .create_room("handoffs", Some("Task handoffs"), None)
        .await
        .unwrap();
    creator.join_room(&room.room_id).await.unwrap();

    let malformed = creator
        .send_message_with_metadata(
            &room.room_id,
            "Malformed handoff remains a normal message",
            None,
            vec![],
            serde_json::json!({
                "kind": "checkpoint"
            }),
        )
        .await
        .unwrap();
    let invalid_acceptance = run(
        &socket,
        &key,
        &["handoff", "accept", "handoffs", &malformed.message_id],
    )
    .await;
    assert!(!invalid_acceptance.status.success());
    assert!(String::from_utf8_lossy(&invalid_acceptance.stderr).contains("not a valid handoff"));

    let created = run(
        &socket,
        &key,
        &[
            "--name",
            "builder",
            "--agent-id",
            "builder-task",
            "handoff",
            "send",
            "handoffs",
            "--task",
            "AUTH-118",
            "--revision",
            "r1",
            "--summary",
            "Auth change is complete",
            "--next",
            "Review expiry-path coverage",
            "--risk",
            "Expiry test is missing",
            "--ref",
            "git:abc123",
        ],
    )
    .await;
    assert!(
        created.status.success(),
        "handoff send failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(String::from_utf8_lossy(&created.stdout).contains("Handoff ready"));

    let listed = run(&socket, &key, &["handoff", "list", "handoffs", "--json"]).await;
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let ready = listed["handoffs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handoff| handoff["kind"] == "handoff.ready")
        .unwrap();
    let handoff_id = ready["message_id"].as_str().unwrap().to_string();
    assert_eq!(ready["state"], "pending");
    assert_eq!(ready["handoff"]["task_id"], "AUTH-118");
    assert_eq!(ready["handoff"]["revision"], "r1");
    assert_eq!(ready["handoff"]["summary"], "Auth change is complete");
    assert_eq!(ready["handoff"]["next"], "Review expiry-path coverage");
    assert_eq!(ready["handoff"]["refs"][0], "git:abc123");

    let cross_task_supersession = run(
        &socket,
        &key,
        &[
            "handoff",
            "send",
            "handoffs",
            "--task",
            "OTHER-9",
            "--revision",
            "r2",
            "--supersedes",
            &handoff_id,
            "--summary",
            "Wrong task",
            "--next",
            "Do not publish",
        ],
    )
    .await;
    assert!(!cross_task_supersession.status.success());
    assert!(String::from_utf8_lossy(&cross_task_supersession.stderr)
        .contains("superseded handoff belongs to task AUTH-118, not OTHER-9"));

    let raw_cross_task_supersession = creator
        .send_message_with_metadata(
            &room.room_id,
            "invalid cross-task replacement",
            None,
            vec![],
            serde_json::json!({
                "kind": "handoff.ready",
                "handoff": {
                    "version": 2,
                    "task_id": "OTHER-9",
                    "revision": "r2",
                    "supersedes": handoff_id,
                    "summary": "Wrong task",
                    "next": "Do not publish",
                    "risks": [],
                    "refs": []
                }
            }),
        )
        .await;
    assert!(raw_cross_task_supersession.is_err());

    let forged_acceptance = creator
        .send_message_with_metadata(
            &room.room_id,
            "forged acceptance",
            Some(&handoff_id),
            vec![],
            serde_json::json!({
                "kind": "handoff.accepted",
                "handoff": {
                    "version": 2,
                    "accepted_handoff_id": handoff_id,
                    "note": "bypass"
                }
            }),
        )
        .await;
    assert!(forged_acceptance.is_err());

    let replacement = run(
        &socket,
        &key,
        &[
            "--name",
            "builder",
            "--agent-id",
            "builder-task",
            "handoff",
            "send",
            "handoffs",
            "--task",
            "AUTH-118",
            "--revision",
            "r2",
            "--supersedes",
            &handoff_id,
            "--summary",
            "Auth change is ready for review",
            "--next",
            "Review expiry-path coverage",
            "--ref",
            "git:def456",
        ],
    )
    .await;
    assert!(
        replacement.status.success(),
        "replacement send failed: {}",
        String::from_utf8_lossy(&replacement.stderr)
    );

    let pending = run(
        &socket,
        &key,
        &["handoff", "list", "handoffs", "--pending", "--json"],
    )
    .await;
    assert!(pending.status.success());
    let pending: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    let pending = pending["handoffs"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    let replacement_id = pending[0]["message_id"].as_str().unwrap().to_string();
    assert_eq!(pending[0]["handoff"]["revision"], "r2");
    assert_eq!(pending[0]["handoff"]["supersedes"], handoff_id);

    let superseded_acceptance = run(
        &socket,
        &key,
        &[
            "--name",
            "reviewer",
            "--agent-id",
            "task-reviewer",
            "handoff",
            "accept",
            "handoffs",
            &handoff_id,
        ],
    )
    .await;
    assert!(!superseded_acceptance.status.success());
    assert!(String::from_utf8_lossy(&superseded_acceptance.stderr)
        .contains("handoff is no longer pending"));

    let accepted = run(
        &socket,
        &key,
        &[
            "--name",
            "reviewer",
            "--agent-id",
            "reviewer-task",
            "handoff",
            "accept",
            "handoffs",
            &replacement_id,
            "--note",
            "Starting review now",
        ],
    )
    .await;
    assert!(
        accepted.status.success(),
        "handoff accept failed: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let duplicate = run(
        &socket,
        &key,
        &[
            "--name",
            "second-reviewer",
            "--agent-id",
            "second-reviewer-task",
            "handoff",
            "accept",
            "handoffs",
            &replacement_id,
        ],
    )
    .await;
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("handoff is no longer pending"));

    let listed = run(&socket, &key, &["handoff", "list", "handoffs", "--json"]).await;
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let accepted = listed["handoffs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handoff| handoff["kind"] == "handoff.accepted")
        .unwrap();
    assert_eq!(accepted["reply_to_message"], replacement_id);
    assert_eq!(accepted["handoff"]["accepted_handoff_id"], replacement_id);
    assert_eq!(accepted["handoff"]["note"], "Starting review now");

    let ready = listed["handoffs"].as_array().unwrap();
    let original = ready
        .iter()
        .find(|handoff| handoff["message_id"] == handoff_id)
        .unwrap();
    assert_eq!(original["state"], "superseded");
    let replacement = ready
        .iter()
        .find(|handoff| handoff["message_id"] == replacement_id)
        .unwrap();
    assert_eq!(replacement["state"], "accepted");
}

#[tokio::test]
async fn concurrent_acceptance_has_exactly_one_owner() {
    let (_server, socket, key, _server_data) = start_test_server().await;
    let creator = CowchatClient::connect_uds(
        &socket,
        &key,
        "handoff-creator",
        Some("handoff-creator"),
        vec![],
    )
    .await
    .unwrap();
    let room = creator
        .create_room("handoffs", Some("Task handoffs"), None)
        .await
        .unwrap();
    creator.join_room(&room.room_id).await.unwrap();

    let created = run(
        &socket,
        &key,
        &[
            "--name",
            "builder",
            "--agent-id",
            "builder-task",
            "handoff",
            "send",
            "handoffs",
            "--task",
            "AUTH-118",
            "--revision",
            "r1",
            "--summary",
            "Ready",
            "--next",
            "Review",
        ],
    )
    .await;
    assert!(created.status.success());
    let pending = run(
        &socket,
        &key,
        &["handoff", "list", "handoffs", "--pending", "--json"],
    )
    .await;
    let pending: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    let handoff_id = pending["handoffs"][0]["message_id"]
        .as_str()
        .unwrap()
        .to_string();

    let first_args = [
        "--name",
        "reviewer-a",
        "--agent-id",
        "reviewer-a-task",
        "handoff",
        "accept",
        "handoffs",
        handoff_id.as_str(),
    ];
    let second_args = [
        "--name",
        "reviewer-b",
        "--agent-id",
        "reviewer-b-task",
        "handoff",
        "accept",
        "handoffs",
        handoff_id.as_str(),
    ];
    let (first, second) = tokio::join!(
        run(&socket, &key, &first_args),
        run(&socket, &key, &second_args)
    );
    assert_eq!(
        usize::from(first.status.success()) + usize::from(second.status.success()),
        1
    );

    let pending = run(
        &socket,
        &key,
        &["handoff", "list", "handoffs", "--pending", "--json"],
    )
    .await;
    let pending: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    assert!(pending["handoffs"].as_array().unwrap().is_empty());
}
