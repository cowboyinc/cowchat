//! Multi-actor coordination over Cowchat rooms, simulating cowboy-repo
//! examples: the message ring (gallery/advanced-messaging-ring), the
//! job/validate/settle pipeline (core/04-multi-actor-workflow), the multisig
//! quorum (core/13-multisig-treasury), and casino rounds
//! (gallery/casino-rounds). Every seat is a real `cowchat actor-host` process
//! running examples/python/coordination_actor.py; actors trigger each other by
//! mentioning the next seat in their replies.
use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

const ROOM_KEY: &str = "coordination-room-secret";

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

async fn owner(net: &Net, room_name: &str) -> (CowchatClient, String) {
    let mut chad = CowchatClient::connect_tcp(&net.addr, &net.key, "Chad", Some("chad"), vec![])
        .await
        .unwrap();
    chad.set_room_secret(ROOM_KEY.as_bytes());
    let room = chad
        .create_room_with_options(room_name, None, None, false, true)
        .await
        .unwrap();
    chad.join_room(&room.room_id).await.unwrap();
    (chad, room.room_id)
}

/// A seat: agent identity, fixture role arguments, subscription mode, state file.
async fn start_seat(
    net: &Net,
    room: &str,
    agent: &str,
    role_args: &[&str],
    mode: &str,
    state: Option<&std::path::Path>,
) -> tokio::process::Child {
    let wake = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = wake.local_addr().unwrap().to_string();
    drop(wake);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/python/coordination_actor.py");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"));
    command
        .args([
            "--tcp",
            &net.addr,
            "--key",
            &net.key,
            "--name",
            agent,
            "--agent-id",
            agent,
            "actor-host",
            room,
            "--listen",
            &port,
            "--mode",
            mode,
            "--",
            "python3",
        ])
        .arg(fixture)
        .args(role_args)
        .env("COWCHAT_ROOM_KEY", ROOM_KEY)
        .env("COWCHAT_WAKE_SECRET", format!("wake-{agent}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(state) = state {
        command.env("COORD_STATE", state);
    }
    let mut child = command.spawn().unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["status"],
        "ready"
    );
    child
}

/// Wait until some message satisfies the predicate; return all decrypted
/// (author, content) pairs at that point.
async fn wait_for(
    client: &CowchatClient,
    room: &str,
    what: &str,
    predicate: impl Fn(&str, &str) -> bool,
) -> Vec<(String, String)> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let all: Vec<(String, String)> = client
                .get_history(room, 200, None)
                .await
                .unwrap()
                .into_iter()
                .map(|m| (m.agent_id, m.content.trim().to_owned()))
                .collect();
            if all.iter().any(|(a, c)| predicate(a, c)) {
                return all;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// gallery/advanced-messaging-ring: a token hops seat-to-seat purely through
/// reply mentions until its hop budget is spent.
#[tokio::test]
async fn message_ring_token_passes_through_reply_mentions() {
    let net = start_server().await;
    let (chad, room) = owner(&net, "ring").await;
    let _a = start_seat(
        &net,
        &room,
        "ring-a",
        &["ring", "a", "ring-b"],
        "addressed",
        None,
    )
    .await;
    let _b = start_seat(
        &net,
        &room,
        "ring-b",
        &["ring", "b", "ring-c"],
        "addressed",
        None,
    )
    .await;
    let _c = start_seat(
        &net,
        &room,
        "ring-c",
        &["ring", "c", "ring-a"],
        "addressed",
        None,
    )
    .await;

    chad.send_message(
        &room,
        r#"{"hops": 4, "path": []}"#,
        None,
        vec!["ring-a".into()],
    )
    .await
    .unwrap();

    let all = wait_for(&chad, &room, "ring completion", |_, c| {
        c.starts_with("ring-complete")
    })
    .await;
    let done: Vec<&(String, String)> = all
        .iter()
        .filter(|(_, c)| c.starts_with("ring-complete"))
        .collect();
    assert_eq!(done.len(), 1, "exactly one completion");
    assert_eq!(done[0].1, "ring-complete path=a>b>c>a>b");
    assert_eq!(done[0].0, "ring-b");
}

/// core/04-multi-actor-workflow: submissions flow submitter -> validator ->
/// settlement ledger; the ledger's durable record is the workflow's outcome.
#[tokio::test]
async fn workflow_pipeline_validates_then_settles() {
    let net = start_server().await;
    let (chad, room) = owner(&net, "workflow").await;
    let state = tempfile::NamedTempFile::new().unwrap();
    let _validator = start_seat(&net, &room, "validator", &["validator"], "addressed", None).await;
    let _ledger = start_seat(
        &net,
        &room,
        "ledger",
        &["ledger"],
        "addressed",
        Some(state.path()),
    )
    .await;

    chad.send_message(&room, "submit:42", None, vec!["validator".into()])
        .await
        .unwrap();
    wait_for(&chad, &room, "first settlement", |a, c| {
        a == "ledger" && c == "recorded #1: valid:42"
    })
    .await;

    chad.send_message(&room, "submit:7", None, vec!["validator".into()])
        .await
        .unwrap();
    wait_for(&chad, &room, "second settlement", |a, c| {
        a == "ledger" && c == "recorded #2: invalid:7"
    })
    .await;

    let ledger = std::fs::read_to_string(state.path()).unwrap();
    assert_eq!(ledger, "record valid:42\nrecord invalid:7\n");
}

/// core/13-multisig-treasury: one proposal fans out to two signers; the
/// treasurer aggregates approvals and executes exactly once at quorum.
#[tokio::test]
async fn multisig_treasury_executes_once_at_quorum() {
    let net = start_server().await;
    let (chad, room) = owner(&net, "treasury").await;
    let state = tempfile::NamedTempFile::new().unwrap();
    let _s1 = start_seat(&net, &room, "sign-1", &["signer", "s1"], "addressed", None).await;
    let _s2 = start_seat(&net, &room, "sign-2", &["signer", "s2"], "addressed", None).await;
    let _t = start_seat(
        &net,
        &room,
        "treasurer",
        &["treasurer"],
        "addressed",
        Some(state.path()),
    )
    .await;

    chad.send_message(
        &room,
        "proposal:p1",
        None,
        vec!["sign-1".into(), "sign-2".into()],
    )
    .await
    .unwrap();

    let all = wait_for(&chad, &room, "execution", |a, c| {
        a == "treasurer" && c == "executed:p1"
    })
    .await;
    assert_eq!(
        all.iter()
            .filter(|(_, c)| c.starts_with("approve:p1"))
            .count(),
        2,
        "both signers approved"
    );
    // Quorum executes exactly once; the first approval was an explicit skip,
    // not a message. The treasurer never says anything else.
    assert_eq!(
        all.iter().filter(|(a, _)| a == "treasurer").count(),
        1,
        "treasurer speaks only to execute"
    );
    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        "approval p1\napproval p1\n"
    );
}

/// gallery/casino-rounds: the casino opens a round, ALWAYS-mode bettors react
/// to the announcement, the feed supplies the price, the casino settles.
#[tokio::test]
async fn casino_round_broadcast_bets_and_settles() {
    let net = start_server().await;
    let (chad, room) = owner(&net, "casino").await;
    let state = tempfile::NamedTempFile::new().unwrap();
    let _casino = start_seat(
        &net,
        &room,
        "casino",
        &["casino"],
        "addressed",
        Some(state.path()),
    )
    .await;
    let _feed = start_seat(&net, &room, "feed", &["feed"], "addressed", None).await;
    let _b1 = start_seat(&net, &room, "bettor-1", &["bettor", "10"], "always", None).await;
    let _b2 = start_seat(&net, &room, "bettor-2", &["bettor", "25"], "always", None).await;

    chad.send_message(&room, "open:r1", None, vec!["casino".into()])
        .await
        .unwrap();

    let all = wait_for(&chad, &room, "settlement", |a, c| {
        a == "casino" && c.starts_with("settled:r1 price=117")
    })
    .await;
    assert_eq!(
        all.iter()
            .filter(|(a, c)| a.starts_with("bettor-") && c.starts_with("bet:r1:"))
            .count(),
        2,
        "both bettors reacted to the round announcement"
    );
    // Bets are recorded in the casino's durable state even when they race the
    // settlement price.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = std::fs::read_to_string(state.path()).unwrap();
            if state.matches("bet bet:r1:").count() == 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("both bets recorded in casino state");
    // The room stays quiet afterwards: skips leave no messages behind.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let quiet = chad.get_history(&room, 200, None).await.unwrap();
    assert!(
        quiet
            .iter()
            .all(|m| !m.content.contains("unknown role") && !m.content.is_empty()),
        "no stray output from skipped work"
    );
}
