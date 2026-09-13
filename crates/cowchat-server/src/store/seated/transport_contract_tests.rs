//! Current SQLite admission/recovery subset. These exercise the signed store
//! entry point, not a model CBQS adapter or an independent archive. Close/reopen
//! proves committed database recovery, not process-kill or storage-loss recovery.
use super::*;

fn install_signed_subscription(store: &Store, now: i64) {
    install_fixture(store);
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM subscriptions WHERE subscription_id='wake'", [])
        .unwrap();
    let target = format!("/rooms/{ROOM}/subscriptions");
    let body = serde_json::to_vec(&serde_json::json!({
        "subscription_id": uuid::Uuid::new_v4().to_string(),
        "transport_generation": 0,
        "webhook_url": "https://example.com/fixture-no-dispatch",
        "secret": "public-fixture-0123456789abcdef0123456789",
        "after": 0
    }))
    .unwrap();
    let (projection, signature) = signed_for("POST", &target, &body, 80, now);
    store
        .subscribe_seated(
            ROOM,
            "fixture-cert",
            "POST",
            &target,
            &body,
            &projection,
            &signature,
            now,
        )
        .unwrap();
}

// All tables are in this test's private database. Include both the delivery
// obligation and its immutable seated wake payload, as well as request nonces.
fn persisted_counts(store: &Store) -> [i64; 5] {
    store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT (SELECT count(*) FROM messages),
                    (SELECT count(*) FROM message_appends),
                    (SELECT count(*) FROM subscription_deliveries),
                    (SELECT count(*) FROM seated_wakes),
                    (SELECT count(*) FROM seated_request_nonces)",
            [],
            |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]),
        )
        .unwrap()
}

fn wake_payload(store: &Store) -> String {
    store
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT payload FROM seated_wakes", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn signed_commit_reopen_preserves_one_logical_record_and_exact_wake() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signed-recovery.db");
    let now = Utc::now().timestamp_millis();
    let body = sealed(b"private signed recovery fixture");
    let (original, dispatch_id, payload) = {
        let store = Store::open(&path).unwrap();
        install_signed_subscription(&store, now);
        let result = append(&store, &body, 1, now).unwrap();
        assert!(result.inserted);
        assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 2]);
        let due = store.load_due_deliveries(Utc::now(), 10).unwrap();
        assert_eq!(due.len(), 1);
        (
            serde_json::to_value(result.message).unwrap(),
            due[0].delivery_id.clone(),
            wake_payload(&store),
        )
        // No worker runs: close after commit, before notification or delivery.
    };
    let store = Store::open(&path).unwrap();
    assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 2]);
    assert!(matches!(
        append(&store, &body, 1, now),
        Err(StoreError::SeatedReplay)
    ));
    let retry = append(&store, &body, 2, now).unwrap();
    assert!(!retry.inserted);
    assert_eq!(serde_json::to_value(retry.message).unwrap(), original);
    assert_eq!(store.room_tip(ROOM).unwrap(), 1);
    assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 3]);
    let due = store.load_due_deliveries(Utc::now(), 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].delivery_id, dispatch_id);
    assert_eq!(due[0].message_id, ID);
    assert_eq!(due[0].message_seq, 1);
    assert_eq!(wake_payload(&store), payload);
    assert!(!payload.contains("private signed recovery fixture"));
    assert!(!original
        .to_string()
        .contains("private signed recovery fixture"));
}

#[test]
fn signed_outbox_failure_reopen_rolls_back_nonce_receipt_position_and_obligation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signed-rollback.db");
    let now = Utc::now().timestamp_millis();
    let body = sealed(b"private signed recovery fixture");
    {
        let store = Store::open(&path).unwrap();
        install_signed_subscription(&store, now);
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_signed_wake BEFORE INSERT ON seated_wakes
             BEGIN SELECT RAISE(ABORT, 'fixture wake failure'); END;",
            )
            .unwrap();
        assert!(append(&store, &body, 1, now).is_err());
        assert_eq!(persisted_counts(&store), [0, 0, 0, 0, 1]);
        assert_eq!(store.room_tip(ROOM).unwrap(), 0);
    }
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(persisted_counts(&store), [0, 0, 0, 0, 1]);
        assert_eq!(store.room_tip(ROOM).unwrap(), 0);
        assert!(store
            .load_due_deliveries(Utc::now(), 10)
            .unwrap()
            .is_empty());
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_signed_wake")
            .unwrap();
        // Exactly the failed request's bytes, timestamp and nonce must succeed.
        let retry = append(&store, &body, 1, now).unwrap();
        assert!(retry.inserted);
        assert_eq!(retry.message.seq, 1);
        assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 2]);
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 2]);
    assert!(matches!(
        append(&store, &body, 1, now),
        Err(StoreError::SeatedReplay)
    ));
    assert!(!append(&store, &body, 2, now).unwrap().inserted);
    assert_eq!(store.room_tip(ROOM).unwrap(), 1);
    assert_eq!(persisted_counts(&store), [1, 1, 1, 1, 3]);
    assert_eq!(store.load_due_deliveries(Utc::now(), 10).unwrap().len(), 1);
}
