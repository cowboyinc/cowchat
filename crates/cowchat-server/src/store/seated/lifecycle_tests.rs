use super::*;
use crate::store::WebhookOutcome;

const SUB: &str = "30000000-0000-4000-8000-000000000099";
fn setup(store: &Store, now: i64) -> Vec<u8> {
    install_fixture(store);
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM subscriptions WHERE subscription_id='wake'", [])
        .unwrap();
    let body = serde_json::to_vec(&serde_json::json!({"subscription_id":SUB,"transport_generation":0,
        "webhook_url":"http://127.0.0.1:9999/wake","secret":"0123456789abcdef0123456789abcdef","after":0})).unwrap();
    let target = format!("/rooms/{ROOM}/subscriptions");
    let (p, s) = signed_for("POST", &target, &body, 1, now);
    store
        .subscribe_seated(ROOM, "fixture-cert", "POST", &target, &body, &p, &s, now)
        .unwrap();
    body
}
fn mutation(kind: &str, revision: i64) -> Vec<u8> {
    let mut action = serde_json::json!({"kind":kind});
    if kind == "update" {
        action["webhook_url"] = "http://127.0.0.1:9998/new".into();
        action["secret"] = "new-secret-new-secret-new-secret-1234".into();
    }
    serde_json::to_vec(
        &serde_json::json!({"operation_id":uuid::Uuid::new_v4().to_string(),
        "transport_generation":0,"expected_revision":revision,"action":action}),
    )
    .unwrap()
}
fn apply(store: &Store, body: &[u8], nonce: u8, now: i64) -> Result<serde_json::Value, StoreError> {
    let target = format!("/rooms/{ROOM}/subscriptions/{SUB}/lifecycle");
    let (p, s) = signed_for("POST", &target, body, nonce, now);
    store.mutate_seated_subscription(
        ROOM,
        SUB,
        "fixture-cert",
        "POST",
        &target,
        body,
        &p,
        &s,
        now,
    )
}
fn mentioned(id: &str) -> Vec<u8> {
    let mut header = crate::seated::SealedRecord::parse(&sealed(b"template"))
        .unwrap()
        .header;
    header.message_id = id.into();
    header.mentions = vec![SEAT.into()];
    seal_header(header, b"private message")
}

#[tokio::test]
async fn seated_subscription_lifecycle_http_exact_retry_delete_and_recreate() {
    let state = crate::web::tests::test_state();
    let store = state.store.clone();
    let now = Utc::now().timestamp_millis();
    let original = setup(&store, now);
    append(&store, &mentioned(ID), 2, now).unwrap();
    let pending = store.load_due_deliveries(Utc::now(), 10).unwrap();
    let old = store
        .seated_wake_payload(SUB, &pending[0].delivery_id, now)
        .unwrap();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let target = format!("/rooms/{ROOM}/subscriptions/{SUB}/lifecycle");
    let http = reqwest::Client::new();
    let post = |body: Vec<u8>, nonce| {
        let (p, s) = signed_for("POST", &target, &body, nonce, now);
        http.post(format!("http://{addr}{target}"))
            .header("x-cowchat-certificate", "fixture-cert")
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(body)
    };
    let update = mutation("update", 0);
    // An unsigned caller cannot mutate or trigger URL validation.
    assert_eq!(
        http.post(format!("http://{addr}{target}"))
            .body(update.clone())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = post(update.clone(), 3).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let receipt: serde_json::Value = response.json().await.unwrap();
    assert_eq!(receipt["revision"], 1);
    assert_eq!(post(update.clone(), 3).send().await.unwrap().status(), 409);
    assert_eq!(
        post(update.clone(), 4)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        store
            .seated_wake_payload(SUB, &pending[0].delivery_id, now)
            .unwrap(),
        old
    );
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.webhook_url,
        "http://127.0.0.1:9998/new"
    );
    let mut changed: serde_json::Value = serde_json::from_slice(&update).unwrap();
    changed["action"]["secret"] = "different-secret-different-secret-123".into();
    assert_eq!(
        post(serde_json::to_vec(&changed).unwrap(), 5)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        post(mutation("repair", 0), 6)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let delete = mutation("delete", 1);
    let response = post(delete.clone(), 7).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let deleted: serde_json::Value = response.json().await.unwrap();
    assert_eq!(deleted["status"], "deleted");
    assert!(store.get_subscription(SUB).unwrap().is_none());
    assert!(store
        .load_due_deliveries(Utc::now(), 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        post(delete, 8)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap(),
        deleted
    );
    // Old operation retries return their historical receipts without resurrecting anything.
    assert_eq!(
        post(update, 9)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap(),
        receipt
    );
    assert!(store.get_subscription(SUB).unwrap().is_none());
    let create_target = format!("/rooms/{ROOM}/subscriptions");
    let (p, s) = signed_for("POST", &create_target, &original, 10, now);
    assert!(matches!(
        store.subscribe_seated(
            ROOM,
            "fixture-cert",
            "POST",
            &create_target,
            &original,
            &p,
            &s,
            now
        ),
        Err(StoreError::MessageConflict)
    ));
    let mut next: serde_json::Value = serde_json::from_slice(&original).unwrap();
    next["subscription_id"] = uuid::Uuid::new_v4().to_string().into();
    let body = serde_json::to_vec(&next).unwrap();
    let (p, s) = signed_for("POST", &create_target, &body, 11, now);
    assert!(store
        .subscribe_seated(
            ROOM,
            "fixture-cert",
            "POST",
            &create_target,
            &body,
            &p,
            &s,
            now
        )
        .is_ok());
    server.abort();
}

#[test]
fn seated_subscription_repair_is_atomic_preserves_wakes_and_fences_old_attempts() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    append(&store, &mentioned(ID), 2, now).unwrap();
    let old = store.load_due_deliveries(Utc::now(), 10).unwrap().remove(0);
    let payload = store
        .seated_wake_payload(SUB, &old.delivery_id, now)
        .unwrap();
    store
        .finish_webhook_attempt(
            &old,
            WebhookOutcome::Abandon {
                reason: "failed",
                fail_subscription: true,
            },
        )
        .unwrap();
    let second = uuid::Uuid::new_v4().to_string();
    append(&store, &mentioned(&second), 3, now).unwrap();
    let repair = mutation("repair", 0);
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_repair BEFORE INSERT ON seated_wakes BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(apply(&store, &repair, 4, now).is_err());
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.status,
        "failed"
    );
    assert!(store
        .load_due_deliveries(Utc::now(), 10)
        .unwrap()
        .is_empty());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_repair")
        .unwrap();
    assert_eq!(apply(&store, &repair, 4, now).unwrap()["revision"], 1); // failed transaction did not consume nonce
    let revived = store.load_due_deliveries(Utc::now(), 10).unwrap().remove(0);
    assert_eq!(old.delivery_id, revived.delivery_id);
    assert_eq!(
        store
            .seated_wake_payload(SUB, &old.delivery_id, now)
            .unwrap(),
        payload
    );
    for outcome in [
        WebhookOutcome::Complete,
        WebhookOutcome::Retry {
            reason: "late",
            next: Utc::now(),
        },
        WebhookOutcome::Abandon {
            reason: "late",
            fail_subscription: true,
        },
    ] {
        assert!(!store.finish_webhook_attempt(&old, outcome).unwrap());
    }
    assert_eq!(
        store
            .get_subscription(SUB)
            .unwrap()
            .unwrap()
            .0
            .last_delivered_seq,
        0
    );
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.status,
        "active"
    );
    assert!(store
        .finish_webhook_attempt(&revived, WebhookOutcome::Complete)
        .unwrap());
    let next = store.load_due_deliveries(Utc::now(), 10).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].message_id, second);
    store.set_subscription_status(SUB, "failed", None).unwrap();
    apply(&store, &repair, 5, now).unwrap(); // exact retry must not repair again
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.status,
        "failed"
    );
}

#[test]
fn seated_subscription_lifecycle_rejects_wrong_scope_and_revoked_authority() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    let body = mutation("repair", 0);
    let target = format!("/rooms/{ROOM}/subscriptions/{SUB}/lifecycle");
    let (p, s) = signed_for("POST", &target, &body, 2, now);
    for (method, uri, bytes) in [
        ("DELETE", target.clone(), body.clone()),
        ("POST", format!("{target}?x=1"), body.clone()),
        ("POST", target.clone(), mutation("delete", 0)),
    ] {
        assert!(store
            .mutate_seated_subscription(
                ROOM,
                SUB,
                "fixture-cert",
                method,
                &uri,
                &bytes,
                &p,
                &s,
                now
            )
            .is_err());
    }
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_subscriptions SET seat='another-seat'", [])
        .unwrap();
    assert!(matches!(
        apply(&store, &body, 2, now),
        Err(StoreError::SeatedAuthorization)
    ));
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_subscriptions SET seat=?1", [SEAT])
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation=2", [])
        .unwrap();
    assert!(matches!(
        apply(&store, &body, 2, now),
        Err(StoreError::SeatedAuthorization)
    ));
}

#[test]
fn seated_subscription_delete_receipt_and_tombstone_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("room.db");
    let now = Utc::now().timestamp_millis();
    let body = mutation("delete", 0);
    let receipt = {
        let store = Store::open(&db).unwrap();
        setup(&store, now);
        apply(&store, &body, 2, now).unwrap()
    };
    let store = Store::open(&db).unwrap();
    assert_eq!(apply(&store, &body, 3, now).unwrap(), receipt);
    assert!(store.get_subscription(SUB).unwrap().is_none());
}

#[tokio::test]
async fn seated_subscription_worker_late_failure_cannot_undo_repair_or_secret_rotation() {
    use axum::{
        body::Bytes,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    let (sent, mut received) = tokio::sync::mpsc::channel(8);
    let release = Arc::new(tokio::sync::Notify::new());
    let release_request = release.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Router::new().route(
        "/wake",
        post(move |headers: HeaderMap, body: Bytes| {
            let (sent, release, count) = (sent.clone(), release_request.clone(), count.clone());
            async move {
                let attempt = count.fetch_add(1, Ordering::SeqCst);
                sent.send((headers, body)).await.unwrap();
                if attempt == 0 {
                    release.notified().await;
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/wake", listener.local_addr().unwrap());
    let sink = tokio::spawn(async move { axum::serve(listener, sink).await.unwrap() });
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE subscriptions SET webhook_url=?1", [&url])
        .unwrap();
    append(&store, &mentioned(ID), 2, now).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE subscription_deliveries SET attempts=5", [])
        .unwrap();
    let manager = crate::webhooks::WebhookManager::new(store.clone(), true);
    let worker = manager.start();
    let first = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    let mut update: serde_json::Value = serde_json::from_slice(&mutation("update", 0)).unwrap();
    update["action"]["webhook_url"] = url.into();
    apply(&store, &serde_json::to_vec(&update).unwrap(), 3, now).unwrap();
    apply(&store, &mutation("repair", 1), 4, now).unwrap();
    release.notify_one(); // failure from revision zero must not fail revision two
    let second = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.1, second.1);
    assert_eq!(first.0["webhook-id"], second.0["webhook-id"]);
    for ((headers, body), secret) in [
        (&first, "0123456789abcdef0123456789abcdef"),
        (&second, "new-secret-new-secret-new-secret-1234"),
    ] {
        assert_eq!(
            headers["webhook-signature"].to_str().unwrap(),
            crate::webhooks::sign_request(
                secret,
                headers["webhook-id"].to_str().unwrap(),
                headers["webhook-timestamp"]
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                std::str::from_utf8(body).unwrap()
            )
        );
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while store
            .get_subscription(SUB)
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
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.status,
        "active"
    );
    assert!(store
        .load_due_deliveries(Utc::now(), 10)
        .unwrap()
        .is_empty());
    worker.abort();
    sink.abort();
}

#[tokio::test]
async fn seated_subscription_update_rechecks_revocation_after_url_validation() {
    use std::sync::Arc;
    let mut state = crate::web::tests::test_state();
    let store = state.store.clone();
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    state.webhook_mgr = crate::webhooks::WebhookManager::new_with_validation_gate(
        store.clone(),
        true,
        started.clone(),
        release.clone(),
    )
    .into();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let body = mutation("update", 0);
    let target = format!("/rooms/{ROOM}/subscriptions/{SUB}/lifecycle");
    let (p, s) = signed_for("POST", &target, &body, 2, now);
    let pending = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("http://{addr}{target}"))
            .header("x-cowchat-certificate", "fixture-cert")
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(body)
            .send()
            .await
            .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation=2", [])
        .unwrap();
    release.notify_one();
    assert_eq!(pending.await.unwrap().status(), 401);
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.webhook_url,
        "http://127.0.0.1:9999/wake"
    );
    server.abort();
}

#[test]
fn seated_subscription_repair_renews_same_seat_and_drops_unreadable_pending_wakes() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    append(&store, &mentioned(ID), 2, now).unwrap();
    // Simulate independently verified renewal under a later readable key floor.
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE seated_credentials SET cert_id='renewed',from_key_generation=3",
            [],
        )
        .unwrap();
    let body = mutation("repair", 0);
    let target = format!("/rooms/{ROOM}/subscriptions/{SUB}/lifecycle");
    let (p, s) = signed_for("POST", &target, &body, 3, now);
    store
        .mutate_seated_subscription(ROOM, SUB, "renewed", "POST", &target, &body, &p, &s, now)
        .unwrap();
    assert!(store
        .load_due_deliveries(Utc::now(), 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.get_subscription(SUB).unwrap().unwrap().0.status,
        "active"
    );
    assert_eq!(
        store
            .get_subscription(SUB)
            .unwrap()
            .unwrap()
            .0
            .last_delivered_seq,
        0
    );
    // A transport migration can delete the old binding, never reinterpret its old wakes.
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET transport_generation=1", [])
        .unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&mutation("delete", 1)).unwrap();
    body["transport_generation"] = 1.into();
    let body = serde_json::to_vec(&body).unwrap();
    let (p, s) = signed_for("POST", &target, &body, 4, now);
    assert!(store
        .mutate_seated_subscription(ROOM, SUB, "renewed", "POST", &target, &body, &p, &s, now)
        .is_ok());
}

#[tokio::test]
async fn seated_worker_transport_failure_does_not_persist_callback_url_credentials() {
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/private-path-canary/wake?token=callback-token-canary",
        listener.local_addr().unwrap()
    );
    let sink = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut discard = [0; 4096];
            let _ = stream.read(&mut discard).await;
            let _ = stream.write_all(b"not-an-http-response\r\n\r\n").await;
        }
    });
    // Reproduce the upstream diagnostic behavior without printing credentials.
    let error = reqwest::Client::new().post(&url).send().await.unwrap_err();
    assert!(error.to_string().contains("callback-token-canary"));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let now = Utc::now().timestamp_millis();
    setup(&store, now);
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE subscriptions SET webhook_url=?1 WHERE subscription_id=?2",
            params![url, SUB],
        )
        .unwrap();
    append(&store, &mentioned(ID), 2, now).unwrap();
    let manager = crate::webhooks::WebhookManager::new(store.clone(), true);
    let worker = manager.start();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row: Option<(i64, String)> = store
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT attempts,last_error FROM subscription_deliveries
                 WHERE subscription_id=?1 AND last_error IS NOT NULL",
                    [SUB],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .unwrap();
            if let Some(row) = row {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    worker.abort();
    sink.abort();
    let (attempts, reason) = result.unwrap();
    assert_eq!(attempts, 1);
    assert_eq!(reason, "transport request failed");
    assert!(!reason.contains("canary"));
    assert!(!reason.contains("http"));
    // The configured URL remains in its intended subscription field.
    let configured: String = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT webhook_url FROM subscriptions WHERE subscription_id=?1",
            [SUB],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(configured, url);
}
