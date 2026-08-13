//! End-to-end CLI tests against an in-process server: the `wait` liveness
//! escape hatches (exit 2 = idle-timeout, 3 = peer ended), the missing-message
//! cursor/drain fix, and the LANTERN overlay flow + reconstruction.

use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::process::Command;
use tokio::time::sleep;

/// Start a test server on a random TCP port; return (handle, tcp_addr, api_key, tmp).
async fn start_test_server() -> (
    tokio::task::JoinHandle<()>,
    String,
    String,
    tempfile::TempDir,
) {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let config = ServerConfig {
        socket_path: tmp_dir.path().join("test.sock"),
        tcp_addr: Some(tcp_addr.clone()),
        http_addr: None,
        db_path: tmp_dir.path().join("test.db"),
        auth_key_path: tmp_dir.path().join("auth.key"),
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
    (handle, tcp_addr, api_key, tmp_dir)
}

/// TCP proxy that deliberately drops its first accepted connection, then
/// transparently forwards every reconnect to the real server.
async fn start_drop_once_proxy(
    upstream: String,
) -> (tokio::task::JoinHandle<()>, String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let connections = Arc::new(AtomicUsize::new(0));
    let observed = connections.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (mut downstream, _) = listener.accept().await.unwrap();
            let mut upstream_stream = tokio::net::TcpStream::connect(&upstream).await.unwrap();
            let attempt = observed.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                if attempt == 0 {
                    tokio::select! {
                        _ = sleep(Duration::from_millis(500)) => {}
                        _ = copy_bidirectional(&mut downstream, &mut upstream_stream) => {}
                    }
                    return;
                }
                let _ = copy_bidirectional(&mut downstream, &mut upstream_stream).await;
            });
        }
    });
    (handle, addr, connections)
}

/// `wait --loop` must reconnect after a transport dies instead of surfacing
/// the request timeout as exit 1. A message sent after reconnection still wakes
/// the original CLI process.
#[tokio::test]
async fn wait_loop_reconnects_after_transport_disconnect() {
    let (_server, server_addr, key, _tmp) = start_test_server().await;
    let (_proxy, proxy_addr, connections) = start_drop_once_proxy(server_addr.clone()).await;

    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &proxy_addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--timeout",
            "1",
            "--since-seq",
            "tip",
            "--idle-timeout",
            "20",
            "--heartbeat-secs",
            "0",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let reconnected = tokio::time::timeout(Duration::from_secs(15), async {
        while connections.load(Ordering::SeqCst) < 2 {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if reconnected.is_err() {
        let status = child.try_wait().unwrap();
        let _ = child.kill().await;
        panic!("wait --loop did not reconnect; child status: {status:?}");
    }

    let speaker = CowchatClient::connect_tcp(&server_addr, &key, "speaker", None, vec![])
        .await
        .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "after reconnect", None, vec![])
        .await
        .unwrap();

    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("reconnected waiter should receive the message")
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("after reconnect"),
        "waiter should print the post-reconnect message"
    );
}

/// Named agent workflows must not silently receive a new server UUID on every
/// one-shot invocation. They either supply an explicit stable ID or inherit one
/// from COWCHAT_AGENT_ID; the unnamed human-oriented CLI remains compatible.
#[tokio::test]
async fn named_agent_command_requires_stable_identity_or_environment() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let missing = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "codex-reviewer",
            "send",
            "lobby",
            "should not send",
        ])
        .env_remove("COWCHAT_AGENT_ID")
        .output()
        .await
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("requires a stable identity"), "{stderr}");
    assert!(stderr.contains("COWCHAT_AGENT_ID"), "{stderr}");

    let from_env = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "codex-reviewer",
            "send",
            "lobby",
            "sent with environment identity",
        ])
        .env("COWCHAT_AGENT_ID", "review-task-4f6f22a8")
        .output()
        .await
        .unwrap();
    assert!(
        from_env.status.success(),
        "environment identity should work: {}",
        String::from_utf8_lossy(&from_env.stderr)
    );

    let from_env_again = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "codex-reviewer",
            "send",
            "lobby",
            "same logical agent on the next process",
        ])
        .env("COWCHAT_AGENT_ID", "review-task-4f6f22a8")
        .output()
        .await
        .unwrap();
    assert!(
        from_env_again.status.success(),
        "the stable environment identity should reconnect: {}",
        String::from_utf8_lossy(&from_env_again.stderr)
    );

    let inspector = CowchatClient::connect_tcp(&addr, &key, "inspector", None, vec![])
        .await
        .unwrap();
    let authored: Vec<_> = inspector
        .get_history("lobby", 20, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| message.agent_name == "codex-reviewer")
        .collect();
    assert_eq!(authored.len(), 2);
    assert!(
        authored
            .iter()
            .all(|message| message.agent_id == "review-task-4f6f22a8"),
        "separate CLI processes must persist one logical agent id: {authored:?}"
    );

    let unnamed = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "send",
            "lobby",
            "ordinary CLI remains compatible",
        ])
        .env_remove("COWCHAT_AGENT_ID")
        .output()
        .await
        .unwrap();
    assert!(
        unnamed.status.success(),
        "default CLI identity should remain optional: {}",
        String::from_utf8_lossy(&unnamed.stderr)
    );

    let named_read = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "codex-reviewer",
            "history",
            "lobby",
        ])
        .env_remove("COWCHAT_AGENT_ID")
        .output()
        .await
        .unwrap();
    assert!(
        named_read.status.success(),
        "read-only named commands remain backward compatible: {}",
        String::from_utf8_lossy(&named_read.stderr)
    );

    let offline = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["--name", "codex-reviewer", "keygen"])
        .env_remove("COWCHAT_AGENT_ID")
        .output()
        .await
        .unwrap();
    assert!(
        offline.status.success(),
        "offline commands must not require connection identity: {}",
        String::from_utf8_lossy(&offline.stderr)
    );
}

/// A `wait --loop` on a silent room exits 2 once the idle deadline passes,
/// instead of blocking forever.
#[tokio::test]
async fn wait_idle_timeout_exits_2() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let status = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new(env!("CARGO_BIN_EXE_cowchat"))
            .args([
                "--tcp",
                &addr,
                "--key",
                &key,
                "--name",
                "waiter",
                "--agent-id",
                "stable-waiter",
                "wait",
                "lobby",
                "--loop",
                "--since-seq",
                "tip",
                "--idle-timeout",
                "1",
                "--heartbeat-secs",
                "0",
            ])
            .status(),
    )
    .await
    .expect("wait should exit on idle, not hang")
    .expect("spawning the cowchat binary should succeed");

    assert_eq!(status.code(), Some(2), "idle-timeout must exit with code 2");
}

/// A peer's `conversation_end` message is surfaced to a waiter, which then exits
/// 3 so its loop terminates rather than waiting for another turn.
#[tokio::test]
async fn wait_conversation_end_exits_3() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    // Waiter: blocking loop, no idle deadline — only a conversation_end (or a
    // real message) should end it. A long idle bound guards against a hang.
    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--since-seq",
            "tip",
            "--idle-timeout",
            "12",
            "--heartbeat-secs",
            "0",
        ])
        .spawn()
        .expect("spawning the cowchat binary should succeed");

    let speaker = CowchatClient::connect_tcp(&addr, &key, "speaker", None, vec![])
        .await
        .unwrap();
    speaker.join_room("lobby").await.unwrap();

    // Wait until the waiter is registered with status "waiting" — the wait
    // command resolves `tip` BEFORE broadcasting that presence, so this is a
    // deterministic barrier (a fixed sleep loses the race on slow starts).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let waiting = speaker
            .list_agents(None)
            .await
            .unwrap()
            .into_iter()
            .any(|a| a.name == "waiter" && a.status.as_deref() == Some("waiting"));
        if waiting {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waiter never reached waiting state"
        );
        sleep(Duration::from_millis(100)).await;
    }
    speaker
        .send_message_with_metadata(
            "lobby",
            "that's a wrap",
            None,
            vec![],
            serde_json::json!({ "kind": "conversation_end" }),
        )
        .await
        .unwrap();

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("waiter should exit after the end message, not hang")
        .expect("waiting on the child should succeed");

    assert_eq!(
        status.code(),
        Some(3),
        "a conversation_end message must make wait exit with code 3"
    );
}

/// The cursor file advances only to messages actually read, never to the
/// agent's own sent seq — so a correction that lands while it "composes" is
/// delivered on the next identical wait, not skipped (the missing-message race).
#[tokio::test]
async fn wait_cursor_file_does_not_skip_unread() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("cursor");
    let cursor_arg = cursor.to_str().unwrap();

    let speaker = CowchatClient::connect_tcp(&addr, &key, "speaker", None, vec![])
        .await
        .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "m1", None, vec![])
        .await
        .unwrap();

    // Turn 1: read m1; the cursor seeds from --since-seq 0 and advances to m1.
    let out = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--cursor-file",
            cursor_arg,
            "--since-seq",
            "0",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "turn 1 should read m1");
    let m1_seq = std::fs::read_to_string(&cursor).unwrap().trim().to_string();

    // Race: a correction lands, then the waiter's OWN reply advances the room tip
    // past it. A `--since-seq tip` loop would now skip the correction.
    speaker
        .send_message("lobby", "correction", None, vec![])
        .await
        .unwrap();
    let waiter_send =
        CowchatClient::connect_tcp(&addr, &key, "waiter", Some("stable-waiter"), vec![])
            .await
            .unwrap();
    waiter_send.join_room("lobby").await.unwrap();
    waiter_send
        .send_message("lobby", "my reply", None, vec![])
        .await
        .unwrap();

    // Turn 2: the identical command. Floor is the cursor (m1), so the correction
    // is delivered (exit 0) and the cursor advances past it — never skipped.
    let out = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--cursor-file",
            cursor_arg,
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "turn 2 must deliver the correction, not time out (the skip bug)"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("correction"),
        "turn 2 should return the correction, got: {stdout}"
    );
    let new_seq = std::fs::read_to_string(&cursor).unwrap().trim().to_string();
    assert_ne!(
        new_seq, m1_seq,
        "cursor must advance past m1 to the correction"
    );
}

/// Backlog catch-up must page past more than one history page of rows that do
/// not wake this logical agent. Otherwise a peer chat immediately after a
/// thinking/self-message burst remains unread while `wait --loop` blocks.
#[tokio::test]
async fn wait_pages_past_filtered_backlog_to_peer_message() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("filtered-backlog-cursor");
    std::fs::write(&cursor, "0").unwrap();

    let waiter = CowchatClient::connect_tcp(&addr, &key, "waiter", Some("stable-waiter"), vec![])
        .await
        .unwrap();
    waiter.join_room("lobby").await.unwrap();
    for index in 0..20 {
        waiter
            .thinking("lobby", &format!("thinking-{index}"))
            .await
            .unwrap();
        waiter
            .send_message("lobby", &format!("self-{index}"), None, vec![])
            .await
            .unwrap();
    }

    let peer = CowchatClient::connect_tcp(&addr, &key, "peer", Some("stable-peer"), vec![])
        .await
        .unwrap();
    peer.join_room("lobby").await.unwrap();
    let peer_message = peer
        .send_message("lobby", "peer-after-filtered-backlog", None, vec![])
        .await
        .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--drain",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "wait must page through filtered backlog instead of timing out: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("peer-after-filtered-backlog"),
        "wait should return the peer message after the filtered prefix: {stdout}"
    );
    assert_eq!(
        stdout.trim().lines().count(),
        1,
        "only peer chat should wake"
    );
    assert_eq!(
        std::fs::read_to_string(&cursor).unwrap().trim(),
        peer_message.seq.to_string(),
        "cursor should advance through the returned peer message"
    );
}

/// `--drain` returns every unread message through the tip in one wait, so a
/// burst that arrived together is processed in a single turn.
#[tokio::test]
async fn wait_drain_returns_full_batch() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("cursor");

    let speaker = CowchatClient::connect_tcp(&addr, &key, "speaker", None, vec![])
        .await
        .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "batch-1", None, vec![])
        .await
        .unwrap();
    speaker
        .send_message("lobby", "batch-2", None, vec![])
        .await
        .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "wait",
            "lobby",
            "--loop",
            "--drain",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--since-seq",
            "0",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("batch-1") && stdout.contains("batch-2"),
        "drain must emit both, got: {stdout}"
    );
    assert_eq!(
        stdout.trim().lines().count(),
        2,
        "drain should emit one line per message"
    );
}

/// `--follow` emits more than one message in a single process, persists its
/// cursor after each row, and terminates cleanly on conversation_end.
#[tokio::test]
async fn wait_follow_streams_multiple_messages_and_persists_cursor() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("follow-cursor");
    let child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "follower",
            "--agent-id",
            "stable-follower",
            "wait",
            "lobby",
            "--follow",
            "--since-seq",
            "0",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--heartbeat-secs",
            "0",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    sleep(Duration::from_millis(500)).await;
    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "follow-one", None, vec![])
        .await
        .unwrap();
    speaker
        .send_message("lobby", "follow-two", None, vec![])
        .await
        .unwrap();
    let end = speaker
        .send_message_with_metadata(
            "lobby",
            "follow-end",
            None,
            vec![],
            serde_json::json!({ "kind": "conversation_end" }),
        )
        .await
        .unwrap();

    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("follow should terminate on conversation_end")
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("follow-one"),
        "missing first message: {stdout}"
    );
    assert!(
        stdout.contains("follow-two"),
        "missing second message: {stdout}"
    );
    assert!(
        stdout.contains("follow-end"),
        "missing end message: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(&cursor).unwrap().trim(),
        end.seq.to_string()
    );
    let leftovers = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(
        leftovers, 0,
        "atomic cursor writes must not leave temp files"
    );
}

/// Drive a full LANTERN thread through the real binary (ASSERT → CHALLENGE →
/// RESOLVE → FUSE), then confirm the read side reconstructs the fused thread and
/// scores both agents' staked claims.
#[tokio::test]
async fn lantern_flow_reconstructs_and_scores() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let bin = env!("CARGO_BIN_EXE_cowchat");
    let run = |name: &'static str, extra: Vec<String>| {
        let addr = addr.clone();
        let key = key.clone();
        async move {
            let mut args = vec![
                "--tcp".to_string(),
                addr,
                "--key".to_string(),
                key,
                "--name".to_string(),
                name.to_string(),
                "--agent-id".to_string(),
                format!("lantern-{name}"),
                "lantern".to_string(),
            ];
            args.extend(extra);
            Command::new(bin).args(&args).output().await.unwrap()
        }
    };

    // ASSERT opens a thread; its seq is the thread id.
    let out = run(
        "aye",
        vec![
            "assert".into(),
            "lobby".into(),
            "--claim".into(),
            "missing rollback gate".into(),
            "--falsifiable-by".into(),
            "a documented operator rollback".into(),
            "--confidence".into(),
            "0.8".into(),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let aseq: i64 = stdout
        .split("seq ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .expect("assert should print its seq");

    // CHALLENGE from the other agent.
    let out = run(
        "bee",
        vec![
            "challenge".into(),
            "lobby".into(),
            "--thread".into(),
            aseq.to_string(),
            "--target-seq".into(),
            aseq.to_string(),
            "--counter-claim".into(),
            "exists as recovery".into(),
            "--confidence".into(),
            "0.6".into(),
            "--test".into(),
            "grep plan".into(),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));
    let cstdout = String::from_utf8_lossy(&out.stdout);
    let cseq: i64 = cstdout
        .split("seq ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap();

    // RESOLVE (scorable basis) + FUSE with calibration outcomes.
    let out = run(
        "aye",
        vec![
            "resolve".into(),
            "lobby".into(),
            "--thread".into(),
            aseq.to_string(),
            "--observation".into(),
            "no operator rollback gate".into(),
            "--basis".into(),
            "artifact".into(),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));
    let out = run(
        "aye",
        vec![
            "fuse".into(),
            "lobby".into(),
            "--thread".into(),
            aseq.to_string(),
            "--synthesis".into(),
            "add the gate".into(),
            "--outcome".into(),
            format!("{aseq}=true"),
            "--outcome".into(),
            format!("{cseq}=false"),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));

    // Reconstruction: the thread is fused.
    let out = run("aye", vec!["threads".into(), "lobby".into()]).await;
    let threads = String::from_utf8_lossy(&out.stdout);
    assert!(
        threads.contains("fused"),
        "thread should be fused, got: {threads}"
    );

    // Calibration scores both agents (aye's assert held, bee's challenge missed).
    let out = run("aye", vec!["calibration".into(), "lobby".into()]).await;
    let cal = String::from_utf8_lossy(&out.stdout);
    assert!(
        cal.contains("aye") && cal.contains("bee"),
        "both agents scored, got: {cal}"
    );
}
