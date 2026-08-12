//! End-to-end CLI tests against an in-process server: the `wait` liveness
//! escape hatches (exit 2 = idle-timeout, 3 = peer ended), the missing-message
//! cursor/drain fix, and the LANTERN overlay flow + reconstruction.

use cowchat_client::CowchatClient;
use cowchat_core::{Frame, FrameType};
use cowchat_server::{CowchatServer, ServerConfig};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncBufReadExt, AsyncWriteExt, BufReader};
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

async fn start_no_auth_test_server() -> (
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
        no_auth: true,
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

fn cursor_seq(path: &std::path::Path) -> i64 {
    let raw = std::fs::read_to_string(path).unwrap();
    raw.trim().parse::<i64>().unwrap_or_else(|_| {
        serde_json::from_str::<serde_json::Value>(&raw).unwrap()["seq"]
            .as_i64()
            .unwrap()
    })
}

fn write_scoped_cursor(
    path: &std::path::Path,
    tcp_addr: &str,
    room_id: &str,
    agent_id: &str,
    seq: i64,
) {
    let endpoint = format!(
        "sha256:{:x}",
        Sha256::digest(format!("tcp:{tcp_addr}").as_bytes())
    );
    std::fs::write(
        path,
        format!(
            "{}\n",
            serde_json::json!({
                "version": 2,
                "endpoint": endpoint,
                "room_id": room_id,
                "agent_id": agent_id,
                "seq": seq,
            })
        ),
    )
    .unwrap();
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

/// Drop the first connection, then make the replacement report a reset room
/// tip. Used to prove an in-memory sequence is revalidated after reconnect.
async fn start_reset_tip_after_drop_proxy(
    upstream: String,
) -> (tokio::task::JoinHandle<()>, String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let connections = Arc::new(AtomicUsize::new(0));
    let observed = connections.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (downstream, _) = listener.accept().await.unwrap();
            let attempt = observed.fetch_add(1, Ordering::SeqCst);
            let upstream_stream = tokio::net::TcpStream::connect(&upstream).await.unwrap();
            tokio::spawn(async move {
                if attempt == 0 {
                    let mut downstream = downstream;
                    let mut upstream_stream = upstream_stream;
                    tokio::select! {
                        _ = sleep(Duration::from_millis(750)) => {}
                        _ = copy_bidirectional(&mut downstream, &mut upstream_stream) => {}
                    }
                    return;
                }

                let (downstream_read, mut downstream_write) = downstream.into_split();
                let (upstream_read, mut upstream_write) = upstream_stream.into_split();
                let requests = tokio::spawn(async move {
                    let mut downstream_read = downstream_read;
                    let _ = tokio::io::copy(&mut downstream_read, &mut upstream_write).await;
                });
                let mut responses = BufReader::new(upstream_read);
                let mut line = String::new();
                loop {
                    line.clear();
                    if responses.read_line(&mut line).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let rendered = match Frame::from_line(&line) {
                        Ok(mut frame) if frame.frame_type == FrameType::RoomTipResult => {
                            frame.payload["seq"] = serde_json::json!(0);
                            frame.to_line().unwrap()
                        }
                        _ => line.clone(),
                    };
                    if downstream_write
                        .write_all(rendered.as_bytes())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                requests.abort();
            });
        }
    });
    (handle, addr, connections)
}

/// Inspect one client connection and close it instead of forwarding the Nth
/// history request. This makes post-wake drain failure deterministic.
async fn start_drop_history_proxy(
    upstream: String,
    drop_on: usize,
) -> (tokio::task::JoinHandle<()>, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = tokio::spawn(async move {
        let (downstream, _) = listener.accept().await.unwrap();
        let upstream = tokio::net::TcpStream::connect(&upstream).await.unwrap();
        let (downstream_read, mut downstream_write) = downstream.into_split();
        let (upstream_read, mut upstream_write) = upstream.into_split();

        let replies = tokio::spawn(async move {
            let mut reader = BufReader::new(upstream_read);
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                if downstream_write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        let mut requests = BufReader::new(downstream_read);
        let mut line = String::new();
        let mut history_requests = 0usize;
        loop {
            line.clear();
            if requests.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            if Frame::from_line(&line).is_ok_and(|frame| frame.frame_type == FrameType::GetHistory)
            {
                history_requests += 1;
                if history_requests == drop_on {
                    break;
                }
            }
            if upstream_write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
        drop(upstream_write);
        replies.abort();
    });
    (handle, addr)
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

#[tokio::test]
async fn wait_rejects_explicit_sequence_ahead_of_initial_tip() {
    let (_server, addr, key, _tmp) = start_test_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "ahead-waiter",
            "--agent-id",
            "stable-ahead-waiter",
            "wait",
            "lobby",
            "--loop",
            "--since-seq",
            "99",
            "--idle-timeout",
            "2",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--since-seq is ahead of room tip (99 > 0)"),
        "unexpected error: {stderr}"
    );
}

#[tokio::test]
async fn wait_rejects_in_memory_sequence_ahead_after_reconnect() {
    let (_server, first_addr, key, tmp) = start_no_auth_test_server().await;
    let speaker = CowchatClient::connect_tcp(
        &first_addr,
        &key,
        "speaker",
        Some("stable-reset-speaker"),
        vec![],
    )
    .await
    .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "establish seq one", None, vec![])
        .await
        .unwrap();

    let (_proxy, proxy_addr, connections) = start_reset_tip_after_drop_proxy(first_addr).await;
    let stderr_path = tmp.path().join("reset-waiter.stderr");
    let stderr_file = std::fs::File::create(&stderr_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &proxy_addr,
            "--key",
            &key,
            "--name",
            "reset-waiter",
            "--agent-id",
            "stable-reset-waiter",
            "wait",
            "lobby",
            "--loop",
            "--timeout",
            "1",
            "--since-seq",
            "1",
            "--idle-timeout",
            "4",
            "--heartbeat-secs",
            "0",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()
        .unwrap();

    let status = match tokio::time::timeout(Duration::from_secs(8), child.wait()).await {
        Ok(status) => status.unwrap(),
        Err(_) => {
            let _ = child.kill().await;
            let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            panic!(
                "wait did not fail after the replacement reset; connections={}, stderr={stderr}",
                connections.load(Ordering::SeqCst)
            );
        }
    };
    assert!(connections.load(Ordering::SeqCst) >= 2);
    assert_eq!(status.code(), Some(1));
    let stderr = std::fs::read_to_string(&stderr_path).unwrap();
    assert!(
        stderr.contains("wait sequence is ahead of room tip (1 > 0)"),
        "unexpected error: {stderr}"
    );
}

/// A bare persistent wait (no caller-supplied cursor/since flag) captures an
/// in-memory history floor and backfills a message posted while reconnecting.
#[tokio::test]
async fn bare_wait_loop_backfills_message_posted_during_disconnect() {
    let (_server, server_addr, key, _tmp) = start_test_server().await;
    let (_proxy, proxy_addr, connections) = start_drop_once_proxy(server_addr.clone()).await;

    let child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &proxy_addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-bare-waiter",
            "wait",
            "lobby",
            "--loop",
            "--timeout",
            "10",
            "--idle-timeout",
            "20",
            "--heartbeat-secs",
            "0",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        while connections.load(Ordering::SeqCst) < 1 {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    sleep(Duration::from_millis(700)).await;
    assert_eq!(connections.load(Ordering::SeqCst), 1);

    let speaker = CowchatClient::connect_tcp(
        &server_addr,
        &key,
        "speaker",
        Some("stable-bare-speaker"),
        vec![],
    )
    .await
    .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "posted while waiter was offline", None, vec![])
        .await
        .unwrap();

    let output = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output())
        .await
        .expect("bare waiter should reconnect and backfill")
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("posted while waiter was offline"));
    assert!(connections.load(Ordering::SeqCst) >= 2);
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

/// Agent-facing CLI commands must not silently register a fresh random UUID.
/// Existing harnesses that omit identity should get an actionable failure.
#[tokio::test]
async fn agent_command_requires_stable_identity_or_environment() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .env_remove("COWCHAT_AGENT_ID")
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "send",
            "lobby",
            "must not be sent",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires a stable identity"));
    assert!(stderr.contains("COWCHAT_AGENT_ID"));

    let named_history = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .env_remove("COWCHAT_AGENT_ID")
        .args([
            "--tcp", &addr, "--key", &key, "--name", "waiter", "history", "lobby",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(named_history.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&named_history.stderr).contains("requires a stable identity"));

    let via_environment = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .env("COWCHAT_AGENT_ID", "stable-env-waiter")
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "send",
            "lobby",
            "sent with an environment identity",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(via_environment.status.code(), Some(0));

    let offline_keygen = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .env_remove("COWCHAT_AGENT_ID")
        .args(["--name", "reporter", "keygen"])
        .output()
        .await
        .unwrap();
    assert_eq!(offline_keygen.status.code(), Some(0));
    assert!(!offline_keygen.stdout.is_empty());
}

/// The canonical cursor-backed command must persist its initial `tip` before
/// blocking. Otherwise a message that lands after an idle timeout but before
/// the identical re-arm is mistaken for old history and skipped.
#[tokio::test]
async fn wait_initializes_cursor_before_timeout_and_catches_rearm_gap() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("rearm-cursor");
    let cursor_arg = cursor.to_str().unwrap();

    let first = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
            "--cursor-file",
            cursor_arg,
            "--since-seq",
            "tip",
            "--idle-timeout",
            "1",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(first.status.code(), Some(2));
    assert_eq!(cursor_seq(&cursor), 0);
    let first_stderr = String::from_utf8_lossy(&first.stderr);
    assert!(first_stderr.contains("Re-run the exact same command"));
    assert!(!first_stderr.contains("Resume with: wait"));

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "follow-up in the re-arm gap", None, vec![])
        .await
        .unwrap();

    let second = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
            "--cursor-file",
            cursor_arg,
            "--since-seq",
            "tip",
            "--idle-timeout",
            "1",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(second.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&second.stdout).contains("follow-up in the re-arm gap"));
    assert_eq!(cursor_seq(&cursor), 1);
}

/// Catch-up checkpoints exactly what it displayed. A message that lands after
/// history but before the first send must remain unread, as must an immediate
/// follow-up that lands before the first wait.
#[tokio::test]
async fn history_cursor_preserves_messages_before_first_send_and_wait() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("send-first-cursor");
    let cursor_arg = cursor.to_str().unwrap();

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message(
            "lobby",
            "pending question read during catch-up",
            None,
            vec![],
        )
        .await
        .unwrap();

    let caught_up = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "history",
            "lobby",
            "--cursor-file",
            cursor_arg,
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(caught_up.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&caught_up.stdout).contains("pending question"));
    assert_eq!(cursor_seq(&cursor), 1);

    speaker
        .send_message(
            "lobby",
            "unseen message in the history-to-send gap",
            None,
            vec![],
        )
        .await
        .unwrap();

    // The checkpoint stays at seq 1; send must not advance it over the unseen
    // seq 2 before posting the answer at seq 3.
    let sent = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "send",
            "lobby",
            "answer to the pending question",
            "--cursor-file",
            cursor_arg,
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(sent.status.code(), Some(0));
    assert_eq!(cursor_seq(&cursor), 1);

    speaker
        .send_message("lobby", "reply before first wait", None, vec![])
        .await
        .unwrap();

    let waited = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
            "--cursor-file",
            cursor_arg,
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(waited.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&waited.stdout);
    assert!(stdout.contains("unseen message in the history-to-send gap"));
    assert!(stdout.contains("reply before first wait"));
    assert!(!stdout.contains("pending question"));
    assert_eq!(cursor_seq(&cursor), 4);
}

/// If callers skip the explicit history checkpoint, first send must prefer a
/// duplicate over loss. It seeds zero, never the current tip, so prior peer
/// messages remain available to the first returning wait.
#[tokio::test]
async fn send_missing_cursor_uses_at_least_once_floor() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("send-zero-cursor");
    let cursor_arg = cursor.to_str().unwrap();

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "unread before first send", None, vec![])
        .await
        .unwrap();

    let sent = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "send",
            "lobby",
            "hello",
            "--cursor-file",
            cursor_arg,
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(sent.status.code(), Some(0));
    assert_eq!(cursor_seq(&cursor), 0);

    let waited = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
            "--cursor-file",
            cursor_arg,
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(waited.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&waited.stdout).contains("unread before first send"));
    assert_eq!(
        cursor_seq(&cursor),
        2,
        "drain checkpoints the self-authored row after evaluating and filtering it"
    );
}

/// Cursor corruption and initialization failures must stop before waiting.
/// Treating either as a missing cursor would silently jump to the current tip.
#[tokio::test]
async fn wait_cursor_errors_fail_closed() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let corrupt = tmp.path().join("corrupt-cursor");
    std::fs::write(&corrupt, "not-a-sequence").unwrap();

    let corrupt_result = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            corrupt.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(corrupt_result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&corrupt_result.stderr).contains("invalid cursor file"));

    let unwritable = tmp.path().join("missing-parent").join("cursor");
    let unwritable_result = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            unwritable.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(unwritable_result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unwritable_result.stderr)
        .contains("failed to initialize cursor file"));

    let follow_corrupt = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--cursor-file",
            corrupt.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(follow_corrupt.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&follow_corrupt.stderr).contains("invalid cursor file"));

    let follow_unwritable = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--cursor-file",
            unwritable.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(follow_unwritable.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&follow_unwritable.stderr)
        .contains("failed to initialize follow cursor file"));
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
async fn wait_cursor_file_with_stable_identity_does_not_skip_unread() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("cursor");
    let cursor_arg = cursor.to_str().unwrap();

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
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
    let m1_seq = cursor_seq(&cursor).to_string();

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
    let new_seq = cursor_seq(&cursor).to_string();
    assert_ne!(
        new_seq, m1_seq,
        "cursor must advance past m1 to the correction"
    );
}

/// Catch-up must page past arbitrarily noisy history. A single fixed-size page
/// of thinking/system/self rows must not hide an already-persisted peer reply.
#[tokio::test]
async fn wait_backlog_pages_past_filtered_noise() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("noise-cursor");
    write_scoped_cursor(&cursor, &addr, "lobby", "stable-waiter", 0);

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    for index in 0..40 {
        speaker
            .send_message_with_metadata(
                "lobby",
                &format!("thinking-{index}"),
                None,
                vec![],
                serde_json::json!({ "type": "thinking" }),
            )
            .await
            .unwrap();
    }
    speaker
        .send_message("lobby", "real reply behind the noise", None, vec![])
        .await
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("real reply behind the noise"));
    assert_eq!(cursor_seq(&cursor), 41);
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

/// Drain must keep paging past the server's 500-row page size before it moves
/// the durable cursor to the captured room tip.
#[tokio::test]
async fn wait_drain_pages_through_more_than_500_rows() {
    let (_handle, addr, key, tmp) = start_no_auth_test_server().await;
    let cursor = tmp.path().join("large-drain-cursor");
    write_scoped_cursor(&cursor, &addr, "lobby", "stable-waiter", 0);

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    for index in 1..=510 {
        speaker
            .send_message("lobby", &format!("burst-{index}"), None, vec![])
            .await
            .unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--not-from",
            "waiter",
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
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim().lines().count(), 510);
    assert!(stdout.contains("burst-1"));
    assert!(stdout.contains("burst-510"));
    assert_eq!(cursor_seq(&cursor), 510);
}

/// A post-wake history failure is an error, not an empty page followed by a
/// cursor jump to the live wake.
#[tokio::test]
async fn wait_drain_propagates_history_fetch_failure_without_checkpointing() {
    let (_handle, server_addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("failed-drain-cursor");

    let speaker = CowchatClient::connect_tcp(
        &server_addr,
        &key,
        "speaker",
        Some("stable-speaker"),
        vec![],
    )
    .await
    .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message("lobby", "wake before failed drain", None, vec![])
        .await
        .unwrap();

    let (_proxy, proxy_addr) = start_drop_history_proxy(server_addr, 2).await;
    write_scoped_cursor(&cursor, &proxy_addr, "lobby", "stable-waiter", 0);
    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--drain",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--timeout",
            "3",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(cursor_seq(&cursor), 0);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("wake before failed drain"));
}

/// Rows rejected by CLI-only filters are processed state. Persist each one
/// before blocking again so cancellation at idle timeout cannot replay them.
#[tokio::test]
async fn filtered_wait_persists_progress_before_idle_timeout() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("filtered-timeout-cursor");
    write_scoped_cursor(&cursor, &addr, "lobby", "stable-waiter", 0);

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    for index in 1..=65 {
        speaker
            .send_message_with_metadata(
                "lobby",
                &format!("noise-{index}"),
                None,
                vec![],
                serde_json::json!({ "kind": "checkpoint" }),
            )
            .await
            .unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--only-kind",
            "review_request",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--idle-timeout",
            "1",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let persisted = cursor_seq(&cursor);
    assert!((1..=65).contains(&persisted));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("seq {persisted}")));

    speaker
        .send_message_with_metadata(
            "lobby",
            "desired review request",
            None,
            vec![],
            serde_json::json!({ "kind": "review_request" }),
        )
        .await
        .unwrap();
    let resumed = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--only-kind",
            "review_request",
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
    assert_eq!(resumed.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("desired review request"));
    assert_eq!(cursor_seq(&cursor), 66);
}

/// Cursor-backed history treats --limit as page size, scans through the
/// captured tip, and checkpoints every evaluated row even when filtered out.
#[tokio::test]
async fn history_cursor_pages_and_checkpoints_all_processed_rows() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("history-pagination-cursor");
    write_scoped_cursor(&cursor, &addr, "lobby", "stable-waiter", 0);

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    for index in 1..=75 {
        let metadata = if index == 55 {
            serde_json::json!({ "kind": "review_request" })
        } else {
            serde_json::json!({ "kind": "checkpoint" })
        };
        speaker
            .send_message_with_metadata(
                "lobby",
                &format!("history-{index}"),
                None,
                vec![],
                metadata,
            )
            .await
            .unwrap();
    }

    let filtered = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "history",
            "lobby",
            "--limit",
            "10",
            "--kind",
            "review_request",
            "--cursor-file",
            cursor.to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(filtered.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&filtered.stdout);
    assert!(stdout.contains("history-55"));
    assert_eq!(stdout.trim().lines().count(), 1);
    assert_eq!(cursor_seq(&cursor), 75);

    let remainder = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "history",
            "lobby",
            "--limit",
            "7",
            "--cursor-file",
            cursor.to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(remainder.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&remainder.stdout)
            .trim()
            .lines()
            .count(),
        0
    );
    assert_eq!(cursor_seq(&cursor), 75);

    let no_match_cursor = tmp.path().join("history-no-match-cursor");
    write_scoped_cursor(&no_match_cursor, &addr, "lobby", "stable-waiter", 0);
    let no_match = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "waiter",
            "--agent-id",
            "stable-waiter",
            "history",
            "lobby",
            "--limit",
            "9",
            "--kind",
            "does-not-exist",
            "--cursor-file",
            no_match_cursor.to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(no_match.status.code(), Some(0));
    assert!(no_match.stdout.is_empty());
    assert_eq!(cursor_seq(&no_match_cursor), 75);
}

/// Vote creation resolves exact room names, and an eligible voter that has
/// explicitly left can use a fresh one-shot CLI process to rejoin before cast.
#[tokio::test]
async fn vote_cli_resolves_room_names_and_rejoins_for_cast() {
    let (_handle, addr, key, _tmp) = start_test_server().await;
    let manager =
        CowchatClient::connect_tcp(&addr, &key, "manager", Some("stable-vote-manager"), vec![])
            .await
            .unwrap();
    let room = manager
        .create_room("vote-cli-room", None, None, false)
        .await
        .unwrap();
    manager.join_room(&room.room_id).await.unwrap();

    let by_name = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "creator",
            "--agent-id",
            "stable-vote-creator",
            "vote",
            "create",
            "vote-cli-room",
            "Does name resolution work?",
            "--options",
            "yes",
            "no",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        by_name.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&by_name.stderr)
    );

    let voter = CowchatClient::connect_tcp(&addr, &key, "voter", Some("stable-voter"), vec![])
        .await
        .unwrap();
    voter.join_room(&room.room_id).await.unwrap();
    let vote = manager
        .create_vote(
            &room.room_id,
            "Can an absent eligible voter cast?",
            None,
            vec!["yes".into(), "no".into()],
            Some(60),
        )
        .await
        .unwrap();
    voter.leave_room(&room.room_id).await.unwrap();
    drop(voter);
    sleep(Duration::from_millis(100)).await;

    let cast = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "voter",
            "--agent-id",
            "stable-voter",
            "vote",
            "cast",
            &vote.vote_id,
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        cast.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&cast.stderr)
    );
}

/// Decline and decide both resolve room names and restore membership explicitly;
/// neither depends on the server's short reconnect-membership stash.
#[tokio::test]
async fn election_cli_rejoins_for_decline_and_decide() {
    let (_handle, addr, key, _tmp) = start_test_server().await;

    let starter = CowchatClient::connect_tcp(
        &addr,
        &key,
        "starter",
        Some("stable-election-starter"),
        vec![],
    )
    .await
    .unwrap();
    let decline_room = starter
        .create_room("decline-cli-room", None, None, false)
        .await
        .unwrap();
    starter.join_room(&decline_room.room_id).await.unwrap();
    let candidate =
        CowchatClient::connect_tcp(&addr, &key, "candidate", Some("stable-decliner"), vec![])
            .await
            .unwrap();
    candidate.join_room(&decline_room.room_id).await.unwrap();
    let started = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "cli-starter",
            "--agent-id",
            "stable-election-cli-starter",
            "election",
            "start",
            "decline-cli-room",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        started.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    candidate.leave_room(&decline_room.room_id).await.unwrap();
    drop(candidate);
    sleep(Duration::from_millis(100)).await;

    let declined = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "candidate",
            "--agent-id",
            "stable-decliner",
            "election",
            "decline",
            "decline-cli-room",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        declined.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&declined.stderr)
    );

    let leader = CowchatClient::connect_tcp(&addr, &key, "leader", Some("stable-decider"), vec![])
        .await
        .unwrap();
    let decide_room = leader
        .create_room("decide-cli-room", None, None, false)
        .await
        .unwrap();
    leader.join_room(&decide_room.room_id).await.unwrap();
    leader.elect_leader(&decide_room.room_id).await.unwrap();
    leader.leave_room(&decide_room.room_id).await.unwrap();
    drop(leader);
    sleep(Duration::from_millis(2_500)).await;

    let decided = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &addr,
            "--key",
            &key,
            "--name",
            "leader",
            "--agent-id",
            "stable-decider",
            "election",
            "decide",
            "decide-cli-room",
            "durable one-shot decision",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        decided.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&decided.stderr)
    );
}

/// Drain applies the same filters that selected the wake. It must not re-add
/// older rows that `--only-kind` (or the peer filters) deliberately excluded.
#[tokio::test]
async fn wait_drain_preserves_message_filters() {
    let (_handle, addr, key, tmp) = start_test_server().await;
    let cursor = tmp.path().join("filtered-drain-cursor");
    write_scoped_cursor(&cursor, &addr, "lobby", "stable-waiter", 0);

    let speaker =
        CowchatClient::connect_tcp(&addr, &key, "speaker", Some("stable-speaker"), vec![])
            .await
            .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message_with_metadata(
            "lobby",
            "checkpoint should stay filtered",
            None,
            vec![],
            serde_json::json!({ "kind": "checkpoint" }),
        )
        .await
        .unwrap();
    speaker
        .send_message_with_metadata(
            "lobby",
            "review request should be returned",
            None,
            vec![],
            serde_json::json!({ "kind": "review_request" }),
        )
        .await
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
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
            "--only-kind",
            "review_request",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--since-seq",
            "tip",
            "--idle-timeout",
            "5",
            "--heartbeat-secs",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("review request should be returned"));
    assert!(!stdout.contains("checkpoint should stay filtered"));
    assert_eq!(stdout.trim().lines().count(), 1);
    assert_eq!(cursor_seq(&cursor), 2);
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
    assert_eq!(cursor_seq(&cursor), end.seq);
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

/// `history --follow` uses the same fixed-tip reconnect/backfill engine as a
/// durable wait: transport EOF is not success, live rows honor filters, and an
/// output file receives both the initial catch-up and post-reconnect tail.
#[tokio::test]
async fn history_follow_reconnects_and_appends_live_output() {
    let (_server, server_addr, key, tmp) = start_test_server().await;
    let speaker = CowchatClient::connect_tcp(
        &server_addr,
        &key,
        "speaker",
        Some("stable-history-speaker"),
        vec![],
    )
    .await
    .unwrap();
    speaker.join_room("lobby").await.unwrap();
    speaker
        .send_message_with_metadata(
            "lobby",
            "initial verdict",
            None,
            vec![],
            serde_json::json!({ "kind": "verdict" }),
        )
        .await
        .unwrap();

    let (_proxy, proxy_addr, connections) = start_drop_once_proxy(server_addr).await;
    let output_path = tmp.path().join("history-follow.txt");
    std::fs::write(&output_path, "stale output must be truncated\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args([
            "--tcp",
            &proxy_addr,
            "--key",
            &key,
            "--name",
            "history-follower",
            "--agent-id",
            "stable-history-follower",
            "history",
            "lobby",
            "--follow",
            "--since-seq",
            "0",
            "--kind",
            "verdict",
            "--output",
            output_path.to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), async {
        while connections.load(Ordering::SeqCst) < 2 {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("history follower should reconnect after transport EOF");
    assert_eq!(
        child.try_wait().unwrap(),
        None,
        "transport EOF must not be swallowed as a successful follow exit"
    );

    speaker
        .send_message("lobby", "filtered live row", None, vec![])
        .await
        .unwrap();
    speaker
        .send_message_with_metadata(
            "lobby",
            "live verdict",
            None,
            vec![],
            serde_json::json!({ "kind": "verdict" }),
        )
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rendered = std::fs::read_to_string(&output_path).unwrap_or_default();
            if rendered.contains("initial verdict") && rendered.contains("live verdict") {
                assert!(!rendered.contains("filtered live row"));
                assert!(!rendered.contains("stale output"));
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("history follower should append the filtered post-reconnect tail");

    child.kill().await.unwrap();
    let _ = child.wait().await;
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
                format!("stable-{name}"),
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
