use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};

async fn capture(
    State(sender): State<tokio::sync::mpsc::Sender<(HeaderMap, String)>>,
    headers: HeaderMap,
    body: String,
) -> StatusCode {
    sender.send((headers, body)).await.unwrap();
    StatusCode::OK
}

#[tokio::test]
async fn append_restart_worker_delivers_committed_obligation_without_notify() {
    let (sender, mut received) = tokio::sync::mpsc::channel(8);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let http = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/hook", post(capture))
                .with_state(sender),
        )
        .await
        .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("restart.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .create_subscription(
                "sub",
                "lobby",
                "owner",
                &url,
                "secret",
                &[],
                None,
                None,
                false,
                0,
            )
            .unwrap();
        store
            .insert_message(
                "committed",
                "lobby",
                "sender",
                "Sender",
                "hello",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
    }
    let store = Arc::new(Store::open(&path).unwrap());
    let manager = WebhookManager::new(store.clone(), true);
    let worker = manager.start(); // startup scan is the only trigger, no notify
    let (headers, body) = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    let event: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(event["message"]["message_id"], "committed");
    let timestamp = headers["webhook-timestamp"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        headers["webhook-signature"].to_str().unwrap(),
        sign_request(
            "secret",
            headers["webhook-id"].to_str().unwrap(),
            timestamp,
            &body
        )
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .0
                .last_delivered_seq
                == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(store
        .load_due_deliveries(chrono::Utc::now(), 32)
        .unwrap()
        .is_empty());
    worker.abort();
    http.abort();
}

#[tokio::test]
async fn append_exhausted_webhook_is_terminal_and_releases_retention() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let http = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/hook", post(|| async { StatusCode::SERVICE_UNAVAILABLE })),
        )
        .await
        .unwrap();
    });
    let store = Arc::new(Store::open_in_memory().unwrap());
    store
        .create_subscription(
            "sub",
            "lobby",
            "owner",
            &url,
            "secret",
            &[],
            None,
            None,
            false,
            0,
        )
        .unwrap();
    store
        .insert_message(
            "failed",
            "lobby",
            "sender",
            "Sender",
            "hello",
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    let mut delivery = store
        .load_due_deliveries(chrono::Utc::now(), 32)
        .unwrap()
        .pop()
        .unwrap();
    delivery.attempts = 5;
    let manager = WebhookManager::new(store.clone(), true);
    process_delivery(manager.inner.clone(), delivery).await;
    assert_eq!(
        store.get_subscription("sub").unwrap().unwrap().0.status,
        "failed"
    );
    assert!(store
        .load_due_deliveries(chrono::Utc::now(), 32)
        .unwrap()
        .is_empty());
    assert_eq!(store.purge_messages_by_tier("free", "+1 hour").unwrap(), 1);
    http.abort();
}

#[tokio::test]
async fn mention_wake_lost_http_ack_and_restart_reuse_authenticated_dispatch() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/wake", listener.local_addr().unwrap());
    let (sender, mut received) = tokio::sync::mpsc::channel(2);
    let http = tokio::spawn(async move {
        for attempt in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut headers = HeaderMap::new();
            let mut line = String::new();
            socket.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("POST /wake "));
            loop {
                line.clear();
                socket.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.trim_end().split_once(':').unwrap();
                headers.insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    value.trim().parse().unwrap(),
                );
            }
            let mut body = vec![
                0;
                headers["content-length"]
                    .to_str()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
            ];
            socket.read_exact(&mut body).await.unwrap();
            sender
                .send((headers, String::from_utf8(body).unwrap()))
                .await
                .unwrap();
            if attempt == 1 {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                socket.flush().await.unwrap();
            }
            // First POST was consumed, but its acknowledgement is lost.
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lost-ack.db");
    let first = {
        let store = Arc::new(Store::open(&path).unwrap());
        store
            .create_subscription_with_mention(
                "actor-wake",
                "lobby",
                "owner",
                &url,
                "secret",
                &[],
                None,
                None,
                true,
                0,
                Some("actor"),
            )
            .unwrap();
        store
            .append_message(&crate::store::MessageAppend {
                message_id: "request",
                room_id: "lobby",
                agent_id: "human",
                agent_name: "Human",
                content: "opaque fixture",
                reply_to: None,
                metadata: &serde_json::json!({}),
                mentions: &["actor".into()],
            })
            .unwrap();
        let manager = WebhookManager::new(store.clone(), true);
        let delivery = store
            .load_due_deliveries(chrono::Utc::now(), 32)
            .unwrap()
            .pop()
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(3),
            process_delivery(manager.inner.clone(), delivery),
        )
        .await
        .unwrap();
        let first = received.recv().await.unwrap();
        let retry = store
            .load_due_deliveries(chrono::Utc::now() + chrono::Duration::seconds(2), 32)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(retry.attempts, 1);
        store
            .reschedule_delivery(
                &retry.delivery_id,
                chrono::Utc::now(),
                retry.attempts,
                "lost ack",
            )
            .unwrap();
        first
    }; // Drop all service state, retaining only SQLite.
    let store = Arc::new(Store::open(&path).unwrap());
    let manager = WebhookManager::new(store.clone(), true);
    let worker = manager.start(); // no append/notify after restart
    let second = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.0["webhook-id"], second.0["webhook-id"]);
    assert_eq!(first.1, second.1);
    for (headers, body) in [&first, &second] {
        let id = headers["webhook-id"].to_str().unwrap();
        let event: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(event["dispatch_id"], id);
        assert_eq!(event["message"]["message_id"], "request");
        let timestamp = headers["webhook-timestamp"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            headers["webhook-signature"].to_str().unwrap(),
            sign_request("secret", id, timestamp, body)
        );
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while store
            .get_subscription("actor-wake")
            .unwrap()
            .unwrap()
            .0
            .last_delivered_seq
            != 1
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(store
        .load_due_deliveries(chrono::Utc::now(), 32)
        .unwrap()
        .is_empty());
    worker.abort();
    let _ = worker.await;
    http.await.unwrap();
}
