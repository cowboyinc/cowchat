use super::*;
use serde_json::json;

fn append<'a>(id: &'a str, metadata: &'a serde_json::Value) -> MessageAppend<'a> {
    MessageAppend {
        message_id: id,
        room_id: "lobby",
        agent_id: "sender",
        agent_name: "Sender",
        content: "hello",
        reply_to: None,
        metadata,
        mentions: &[],
    }
}
fn subscribe(store: &Store) {
    store
        .create_subscription(
            "sub",
            "lobby",
            "owner",
            "https://example.com/hook",
            "secret",
            &[],
            None,
            None,
            true,
            0,
        )
        .unwrap();
}

#[test]
fn append_reopen_recovers_committed_outbox_without_notification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("append.db");
    let metadata = json!({"kind":"request"});
    let original = {
        let store = Store::open(&path).unwrap();
        subscribe(&store);
        store
            .append_message(&append("retry", &metadata))
            .unwrap()
            .message
        // No WebhookManager or notify exists: model a crash immediately after commit.
    };
    let store = Store::open(&path).unwrap();
    let due = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].message_id, "retry");
    let mut retry = append("retry", &metadata);
    retry.agent_name = "New display name";
    let replay = store.append_message(&retry).unwrap();
    assert!(!replay.inserted);
    assert_eq!(
        serde_json::to_value(replay.message).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(store.room_tip("lobby").unwrap(), 1);
    assert_eq!(store.load_due_deliveries(Utc::now(), 32).unwrap().len(), 1);
    // A newly-created subscription cannot gain a delivery from retrying an old message.
    store
        .create_subscription(
            "later",
            "lobby",
            "owner",
            "https://example.com/new",
            "secret",
            &[],
            None,
            None,
            false,
            1,
        )
        .unwrap();
    store.append_message(&retry).unwrap();
    assert_eq!(store.load_due_deliveries(Utc::now(), 32).unwrap().len(), 1);
}

#[test]
fn append_concurrent_connections_allocate_once_and_conflict_on_changed_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.db");
    let stores: Vec<_> = (0..6).map(|_| Store::open(&path).unwrap()).collect();
    subscribe(&stores[0]);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(stores.len()));
    let joins: Vec<_> = stores
        .into_iter()
        .map(|store| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .append_message(&append("race", &json!({})))
                    .unwrap()
                    .inserted
            })
        })
        .collect();
    assert_eq!(
        joins
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let store = Store::open(&path).unwrap();
    assert_eq!(store.room_tip("lobby").unwrap(), 1);
    for field in ["content", "room", "agent", "reply", "mentions", "metadata"] {
        let metadata = if field == "metadata" {
            json!({"changed":true})
        } else {
            json!({})
        };
        let mentions = vec!["other".to_string()];
        let mut request = append("race", &metadata);
        match field {
            "content" => request.content = "different",
            "room" => request.room_id = "elsewhere",
            "agent" => request.agent_id = "other",
            "reply" => request.reply_to = Some("other"),
            "mentions" => request.mentions = &mentions,
            _ => {}
        }
        assert!(
            matches!(
                store.append_message(&request),
                Err(StoreError::MessageConflict)
            ),
            "{field}"
        );
    }
    assert_eq!(store.room_tip("lobby").unwrap(), 1);
}

#[test]
fn append_outbox_failure_rolls_back_receipt_message_and_sequence() {
    let store = Store::open_in_memory().unwrap();
    subscribe(&store);
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_outbox BEFORE INSERT ON subscription_deliveries
         BEGIN SELECT RAISE(ABORT, 'injected enqueue failure'); END;",
        )
        .unwrap();
    let metadata = json!({});
    assert!(store.append_message(&append("failed", &metadata)).is_err());
    assert!(store.get_message("failed").unwrap().is_none());
    assert!(store
        .replay_append(&append("failed", &metadata))
        .unwrap()
        .is_none());
    assert_eq!(store.room_tip("lobby").unwrap(), 0);
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_outbox")
        .unwrap();
    assert_eq!(
        store
            .append_message(&append("failed", &metadata))
            .unwrap()
            .message
            .seq,
        1
    );
}

#[test]
fn append_receipt_survives_history_but_has_bounded_horizon_without_content() {
    let store = Store::open_in_memory().unwrap();
    let metadata = json!({"sensitive":"not kept in receipt"});
    let original = store
        .append_message(&append("old", &metadata))
        .unwrap()
        .message;
    // Age the stored row, leaving the receipt's actual original timestamp intact.
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET created_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
    assert_eq!(store.purge_messages_by_tier("free", "-14 days").unwrap(), 1);
    assert!(store.get_message("old").unwrap().is_none());
    let replay = store.append_message(&append("old", &metadata)).unwrap();
    assert!(!replay.inserted);
    assert_eq!(
        serde_json::to_value(replay.message).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    let conn = store.conn.lock().unwrap();
    let mut columns = conn.prepare("PRAGMA table_info(message_appends)").unwrap();
    let names: Vec<String> = columns
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        names,
        [
            "message_id",
            "room_id",
            "agent_id",
            "append_digest",
            "agent_name",
            "created_at",
            "seq"
        ]
    );
    drop(columns);
    conn.execute(
        "UPDATE message_appends SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-20 days')",
        [],
    )
    .unwrap();
    drop(conn);
    store.purge_messages_by_tier("free", "-14 days").unwrap();
    assert!(store
        .replay_append(&append("old", &metadata))
        .unwrap()
        .is_some());
    store.conn.lock().unwrap().execute("UPDATE message_appends SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-22 days')", []).unwrap();
    store.purge_messages_by_tier("free", "-14 days").unwrap();
    assert!(store
        .replay_append(&append("old", &metadata))
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .append_message(&append("old", &metadata))
            .unwrap()
            .message
            .seq,
        2
    );
}

#[test]
fn append_pending_delivery_pins_retention_until_deadline_and_records_abandonment() {
    let store = Store::open_in_memory().unwrap();
    subscribe(&store);
    store
        .append_message(&append("pending", &json!({})))
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET created_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
    assert_eq!(store.purge_messages_by_tier("free", "-14 days").unwrap(), 0);
    store.conn.lock().unwrap().execute("UPDATE subscription_deliveries SET deadline_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 second')", []).unwrap();
    assert_eq!(store.purge_messages_by_tier("free", "-14 days").unwrap(), 1);
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
    let status: String = store
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT status FROM subscription_deliveries", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "abandoned");
    assert!(store.earliest_pending_attempt().unwrap().is_none());
}

#[test]
fn append_filters_thinking_system_and_prior_generation_rows_fail_closed() {
    let store = Store::open_in_memory().unwrap();
    subscribe(&store);
    store
        .insert_message(
            "thought",
            "lobby",
            "sender",
            "Sender",
            "thinking",
            None,
            &json!({"type":"thinking"}),
        )
        .unwrap();
    store
        .insert_message(
            "system",
            "lobby",
            "sender",
            "Sender",
            "system",
            None,
            &json!({"type":"system"}),
        )
        .unwrap();
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
    store
        .append_message(&append("pre-upgrade", &json!({})))
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM message_appends WHERE message_id = 'pre-upgrade'",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.append_message(&append("pre-upgrade", &json!({}))),
        Err(StoreError::MessageConflict)
    ));
    store.delete_room_artifacts("lobby").unwrap();
    assert!(store
        .replay_append(&append("thought", &json!({"type":"thinking"})))
        .unwrap()
        .is_none());
}

#[test]
fn append_upgrade_adds_receipts_and_bounds_legacy_pending_deliveries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.db");
    {
        let store = Store::open(&path).unwrap();
        subscribe(&store);
        store.append_message(&append("legacy", &json!({}))).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch(
                "DROP TABLE message_appends;
             DROP INDEX idx_deliveries_message_pending;
             DROP INDEX idx_deliveries_deadline;
             ALTER TABLE subscription_deliveries DROP COLUMN status;
             ALTER TABLE subscription_deliveries DROP COLUMN deadline_at;",
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.room_tip("lobby").unwrap(), 1);
    assert_eq!(store.load_due_deliveries(Utc::now(), 32).unwrap().len(), 1);
    assert!(matches!(
        store.append_message(&append("legacy", &json!({}))),
        Err(StoreError::MessageConflict)
    ));
    // A migrated obligation expires after 24h even if the endpoint never runs.
    assert!(store
        .load_due_deliveries(Utc::now() + chrono::Duration::hours(25), 32)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .append_message(&append("new", &json!({})))
            .unwrap()
            .message
            .seq,
        2
    );
}

#[test]
fn mention_subscription_filters_explicit_ids_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mentions.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .create_subscription_with_mention(
                "mention-sub",
                "lobby",
                "owner",
                "https://example.com/hook",
                "secret",
                &[],
                None,
                None,
                true,
                0,
                Some("actor"),
            )
            .unwrap();
        let spoof = json!({"mentions":["actor"]});
        let mut input = append("unmentioned", &spoof);
        input.content = "@actor this is only text";
        store.append_message(&input).unwrap();
        let mentions = vec!["different-actor".to_string()];
        input.message_id = "wrong-target";
        input.mentions = &mentions;
        store.append_message(&input).unwrap();
        assert!(store
            .load_due_deliveries(Utc::now(), 32)
            .unwrap()
            .is_empty());
        let mentions = vec!["actor".to_string(), "actor".to_string()];
        input.message_id = "mentioned";
        input.mentions = &mentions;
        store.append_message(&input).unwrap();
        store.append_message(&input).unwrap();
        let thinking = json!({"type":"thinking"});
        input.metadata = &thinking;
        input.message_id = "thought";
        store.append_message(&input).unwrap();
    }
    let store = Store::open(&path).unwrap();
    let subscription = store.get_subscription("mention-sub").unwrap().unwrap().0;
    assert_eq!(subscription.only_mention.as_deref(), Some("actor"));
    assert_eq!(
        store.list_subscriptions("owner", None).unwrap()[0].only_mention,
        subscription.only_mention
    );
    assert_eq!(
        store.get_message_mentions("mentioned").unwrap(),
        ["actor", "actor"]
    );
    let due = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].message_id, "mentioned");
}

#[test]
fn mention_upgrade_does_not_invent_legacy_mentions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-mentions.db");
    {
        let store = Store::open(&path).unwrap();
        subscribe(&store);
        store
            .append_message(&append("legacy", &json!({"mentions":["actor"]})))
            .unwrap();
        store.conn.lock().unwrap().execute_batch(
            "ALTER TABLE messages DROP COLUMN mentions; ALTER TABLE subscriptions DROP COLUMN only_mention;"
        ).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert!(store.get_message_mentions("legacy").unwrap().is_empty());
    assert!(store
        .get_subscription("sub")
        .unwrap()
        .unwrap()
        .0
        .only_mention
        .is_none());
}

#[test]
fn webhook_acknowledgement_failure_preserves_delivery_and_cursor_together() {
    let store = Store::open_in_memory().unwrap();
    subscribe(&store);
    store.append_message(&append("ack", &json!({}))).unwrap();
    let original = store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .pop()
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_ack BEFORE DELETE ON subscription_deliveries
         BEGIN SELECT RAISE(ABORT, 'injected acknowledgement failure'); END;",
        )
        .unwrap();
    assert!(store.complete_delivery(&original.delivery_id).is_err());
    assert_eq!(
        store
            .get_subscription("sub")
            .unwrap()
            .unwrap()
            .0
            .last_delivered_seq,
        0
    );
    assert_eq!(
        store.load_due_deliveries(Utc::now(), 32).unwrap()[0].delivery_id,
        original.delivery_id
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_ack")
        .unwrap();
    store.complete_delivery(&original.delivery_id).unwrap();
    store.complete_delivery(&original.delivery_id).unwrap();
    assert_eq!(
        store
            .get_subscription("sub")
            .unwrap()
            .unwrap()
            .0
            .last_delivered_seq,
        1
    );
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
}

#[test]
fn mention_subscription_backfill_is_atomic_and_completed_wake_cannot_be_recreated() {
    let store = Store::open_in_memory().unwrap();
    let metadata = json!({});
    let mentions = vec!["actor".into()];
    let mut request = append("past", &metadata);
    request.mentions = &mentions;
    store.append_message(&request).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_initial_backfill BEFORE INSERT ON subscription_deliveries
         BEGIN SELECT RAISE(ABORT, 'injected initial backlog failure'); END;",
        )
        .unwrap();
    assert!(store
        .create_subscription_with_mention(
            "atomic-sub",
            "lobby",
            "owner",
            "https://example.com/hook",
            "secret",
            &[],
            None,
            None,
            true,
            0,
            Some("actor")
        )
        .is_err());
    assert!(store.get_subscription("atomic-sub").unwrap().is_none());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_initial_backfill")
        .unwrap();
    store
        .create_subscription_with_mention(
            "atomic-sub",
            "lobby",
            "owner",
            "https://example.com/hook",
            "secret",
            &[],
            None,
            None,
            true,
            0,
            Some("actor"),
        )
        .unwrap();
    request.message_id = "live";
    store.append_message(&request).unwrap();
    let first = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].message_id, "past");
    store.complete_delivery(&first[0].delivery_id).unwrap();
    // A stale backfill snapshot must not invent a second dispatch after an ack.
    assert!(!store
        .enqueue_delivery("different-dispatch", "atomic-sub", 1, "past", Utc::now())
        .unwrap());
    let next = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].message_id, "live");
}

#[test]
fn mention_reenable_preserves_failed_wake_identity_and_atomically_recovers_backlog() {
    let store = Store::open_in_memory().unwrap();
    store
        .create_subscription_with_mention(
            "repair",
            "lobby",
            "owner",
            "https://example.com/hook",
            "secret",
            &[],
            None,
            None,
            true,
            0,
            Some("actor"),
        )
        .unwrap();
    let metadata = json!({});
    let mentions = vec!["actor".into()];
    let mut request = append("failed-wake", &metadata);
    request.mentions = &mentions;
    store.append_message(&request).unwrap();
    let delivery = store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .pop()
        .unwrap();
    store
        .abandon_delivery(&delivery.delivery_id, "retry exhausted")
        .unwrap();
    store
        .set_subscription_status("repair", "failed", Some(6))
        .unwrap();
    request.message_id = "while-failed";
    store.append_message(&request).unwrap();
    request.message_id = "not-for-actor";
    request.mentions = &[];
    store.append_message(&request).unwrap();
    assert!(!store
        .enable_subscription_with_backfill("repair", "outsider")
        .unwrap());
    assert_eq!(
        store.get_subscription("repair").unwrap().unwrap().0.status,
        "failed"
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_reenable BEFORE UPDATE OF status ON subscriptions
         BEGIN SELECT RAISE(ABORT, 'injected repair failure'); END;",
        )
        .unwrap();
    assert!(store
        .enable_subscription_with_backfill("repair", "owner")
        .is_err());
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.get_subscription("repair").unwrap().unwrap().0.status,
        "failed"
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_reenable")
        .unwrap();
    assert!(store
        .enable_subscription_with_backfill("repair", "owner")
        .unwrap());
    let first = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].delivery_id, delivery.delivery_id);
    store.complete_delivery(&first[0].delivery_id).unwrap();
    let next = store.load_due_deliveries(Utc::now(), 32).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].message_id, "while-failed");
    store.complete_delivery(&next[0].delivery_id).unwrap();
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
    assert!(store
        .enable_subscription_with_backfill("repair", "owner")
        .unwrap());
    assert!(store
        .load_due_deliveries(Utc::now(), 32)
        .unwrap()
        .is_empty());
}
