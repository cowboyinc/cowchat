//! Real CLI processes against an authenticated server.
use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::time::Duration;

struct Net {
    addr: String,
    key: String,
    _dir: tempfile::TempDir,
}

async fn start_server() -> Net {
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
        max_rooms: None,
    };
    let server = CowchatServer::new(config).unwrap();
    let key = server.api_key().to_owned();
    tokio::spawn(async move { server.run().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    Net {
        addr,
        key,
        _dir: dir,
    }
}

fn cli(net: &Net) -> tokio::process::Command {
    cli_as(net, "lease-bot")
}
fn cli_as(net: &Net, id: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"));
    cmd.args([
        "--tcp",
        &net.addr,
        "--key",
        &net.key,
        "--name",
        id,
        "--agent-id",
        id,
        "lease",
    ]);
    cmd
}
async fn output(net: &Net, args: &[&str]) -> serde_json::Value {
    let mut cmd = if matches!(args.first(), Some(&"status") | Some(&"list")) {
        cli_as(net, "observer")
    } else {
        cli(net)
    };
    let output = cmd.args(args).output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[tokio::test]
async fn lease_across_cli_connections_and_supervised_child() {
    let net = start_server().await;
    let owner = CowchatClient::connect_tcp(&net.addr, &net.key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room = owner.create_room("lease-test", None, None).await.unwrap();
    assert_eq!(
        output(&net, &["acquire", &room.room_id, "deploy:test"]).await["lease"]["generation"],
        1
    );
    for action in ["heartbeat", "release"] {
        let rejected = cli_as(&net, "other-agent")
            .args([action, &room.room_id, "deploy:test", "--generation", "1"])
            .output()
            .await
            .unwrap();
        assert!(!rejected.status.success());
    }
    assert_eq!(
        output(
            &net,
            &[
                "heartbeat",
                &room.room_id,
                "deploy:test",
                "--generation",
                "1",
                "--state",
                "waiting-for-human",
                "--detail",
                "Approve deploy"
            ]
        )
        .await["lease"]["task_state"],
        "waiting_for_human"
    );
    assert_eq!(
        output(
            &net,
            &[
                "validate",
                &room.room_id,
                "deploy:test",
                "--generation",
                "1"
            ]
        )
        .await["valid"],
        true
    );
    assert_eq!(
        output(
            &net,
            &["release", &room.room_id, "deploy:test", "--generation", "1"]
        )
        .await["lease"]["status"],
        "released"
    );
    let state = net._dir.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"state":"waiting_for_dependency","progress":"Build queued"}"#,
    )
    .unwrap();
    let result = cli(&net)
        .args([
            "run",
            &room.room_id,
            "deploy:test",
            "--state-file",
            state.to_str().unwrap(),
            "--",
            "sh",
            "-c",
            "echo child-output; exit 7",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(result.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "child-output\n");
    let status = output_status(&net, &room.room_id).await;
    assert_eq!(status["lease"]["generation"], 2);
    assert_eq!(status["lease"]["status"], "released");
    assert_eq!(status["lease"]["task_state"], "waiting_for_dependency");
    let retry = cli(&net)
        .args([
            "heartbeat",
            &room.room_id,
            "deploy:test",
            "--generation",
            "1",
        ])
        .output()
        .await
        .unwrap();
    assert!(!retry.status.success());
}
async fn output_status(net: &Net, room: &str) -> serde_json::Value {
    output(net, &["status", room, "deploy:test"]).await
}
#[tokio::test]
async fn coordination_enforces_private_room_access_and_join() {
    use cowchat_core::FrameType;
    use serde_json::json;
    let net = start_server().await;
    let owner = CowchatClient::connect_tcp(&net.addr, &net.key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room = owner
        .create_room("private-lease-test", None, None)
        .await
        .unwrap();
    let request = |op| json!({"room_id":room.room_id,"operation":op});
    let acquire = json!({"action":"acquire","name":"deploy:test","expected_generation":0});
    let error = owner
        .request(FrameType::RoomCoordination, request(acquire.clone()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Join"));
    owner.join_room(&room.room_id).await.unwrap();
    owner
        .request(FrameType::RoomCoordination, request(acquire))
        .await
        .unwrap();
    let outsider_key = "lease-outsider-key";
    rusqlite::Connection::open(net._dir.path().join("server.db"))
        .unwrap()
        .execute(
            "INSERT INTO api_keys(api_key,label) VALUES (?1,'outsider')",
            [outsider_key],
        )
        .unwrap();
    let stranger = CowchatClient::connect_tcp(
        &net.addr,
        outsider_key,
        "Stranger",
        Some("stranger"),
        vec![],
    )
    .await
    .unwrap();
    let error = stranger
        .request(
            FrameType::RoomCoordination,
            request(json!({"action":"list"})),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        cowchat_client::ClientError::Server {
            code: cowchat_core::ErrorCode::AccessDenied,
            ..
        }
    ));
}

#[tokio::test]
async fn supervisor_renews_while_waiting_then_releases_on_signal() {
    let net = start_server().await;
    let owner = CowchatClient::connect_tcp(&net.addr, &net.key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room = owner
        .create_room("supervised-lease-test", None, None)
        .await
        .unwrap();
    let state = net._dir.path().join("host-state.json");
    std::fs::write(&state, r#"{"state":"working","progress":"Starting"}"#).unwrap();
    let mut child = cli(&net)
        .args([
            "run",
            &room.room_id,
            "deploy:test",
            "--state-file",
            state.to_str().unwrap(),
            "--",
            "sleep",
            "120",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            assert!(child.try_wait().unwrap().is_none(), "Supervisor exited before acquiring its lease");
            let status = owner.request(cowchat_core::FrameType::RoomCoordination, serde_json::json!({"room_id":room.room_id,"operation":{"action":"get","name":"deploy:test"}})).await.unwrap().payload;
            if status["lease"]["generation"] == 1 {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    // A real minute must pass: the child is blocked, but the supervisor stays alive.
    std::fs::write(&state,r#"{"state":"waiting_for_human","detail":"Approve command","progress":"Ready for approval"}"#).unwrap();
    tokio::time::sleep(Duration::from_secs(62)).await;
    let status = output_status(&net, &room.room_id).await;
    assert!(
        status["lease"]["last_heartbeat_at"].as_i64().unwrap()
            >= first["lease"]["last_heartbeat_at"].as_i64().unwrap() + 60
    );
    assert_eq!(status["lease"]["task_state"], "waiting_for_human");
    assert_eq!(status["lease"]["progress"], "Ready for approval");
    unsafe {
        libc::kill(child.id().unwrap() as i32, libc::SIGTERM);
    }
    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!exit.success());
    assert_eq!(
        output_status(&net, &room.room_id).await["lease"]["status"],
        "released"
    );
}

#[tokio::test]
async fn supervisor_stops_child_after_losing_lease() {
    let net = start_server().await;
    let owner = CowchatClient::connect_tcp(&net.addr, &net.key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room = owner
        .create_room("lost-lease-test", None, None)
        .await
        .unwrap();
    let marker = net._dir.path().join("should-never-exist");
    let started = net._dir.path().join("started");
    let mut child = cli(&net)
        .args([
            "run",
            &room.room_id,
            "deploy:test",
            "--",
            "sh",
            "-c",
            "touch \"$1\"; sleep 70; touch \"$2\"",
            "lease-test",
        ])
        .arg(&started)
        .arg(&marker)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            assert!(child.try_wait().unwrap().is_none(), "Supervisor exited before acquiring its lease");
            let status = owner.request(cowchat_core::FrameType::RoomCoordination, serde_json::json!({"room_id":room.room_id,"operation":{"action":"get","name":"deploy:test"}})).await.unwrap().payload;
            if status["lease"]["generation"] == 1 && started.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    // Expire the server-side grant without adding a test-only TTL option.
    rusqlite::Connection::open(net._dir.path().join("server.db"))
        .unwrap()
        .execute(
            "UPDATE room_leases SET heartbeat=0 WHERE room=?1",
            [&room.room_id],
        )
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(65), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert!(
        !marker.exists(),
        "The supervised process group must stop after losing ownership"
    );
}
