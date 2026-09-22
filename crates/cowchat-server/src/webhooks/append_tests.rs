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
