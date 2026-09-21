use super::*;
use cowchat_core::{ActorWorkOutcome, WakeMode};
use serde_json::json;

fn subscribe(store: &Store, actor: &str, mode: WakeMode) -> String {
    store
        .create_actor_subscription(
            "lobby",
            "owner",
            actor,
            "https://example.com/wake",
            "secret",
            mode,
        )
        .unwrap()
}
fn send(store: &Store, id: &str, actor: &str, mentions: &[String]) -> AppendResult {
    store
        .append_message(&MessageAppend {
            message_id: id,
            room_id: "lobby",
            agent_id: actor,
            agent_name: actor,
            content: "cow1:opaque",
            reply_to: None,
            metadata: &json!({}),
            mentions,
        })
        .unwrap()
}
fn reply(
    store: &Store,
    work: &cowchat_core::ActorWork,
    actor: &str,
) -> Result<AppendResult, StoreError> {
    store.append_message(&MessageAppend {
        message_id: &work.reply_message_id,
        room_id: "lobby",
        agent_id: actor,
        agent_name: actor,
        content: "cow1:reply",
        reply_to: Some(&work.message_id),
        metadata: &json!({}),
        mentions: &[],
    })
}

#[test]
fn actor_work_restart_serializes_claims_and_reply_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("work.db");
    let sub = {
        let store = Store::open(&path).unwrap();
        let sub = subscribe(&store, "actor", WakeMode::Always);
        send(&store, "first", "human", &[]);
        send(&store, "second", "human", &[]);
        sub
    };
    let store = Store::open(&path).unwrap();
    assert!(store
        .claim_actor_work(&sub, "other", 100)
        .unwrap()
        .is_none());
    let first = store.claim_actor_work(&sub, "actor", 100).unwrap().unwrap();
    assert_eq!(first.message_id, "first");
    assert!(store
        .claim_actor_work(&sub, "actor", 399)
        .unwrap()
        .is_none());
    assert!(store
        .complete_actor_work(&sub, &first.work_id, "actor", ActorWorkOutcome::Replied)
        .is_err());
    assert!(reply(&store, &first, "impostor").is_err());
    let recovered = store.claim_actor_work(&sub, "actor", 400).unwrap().unwrap();
    assert_eq!(first.work_id, recovered.work_id);
    assert!(reply(&store, &first, "actor").unwrap().inserted);
    drop(store); // crash after reply, before ack
    let store = Store::open(&path).unwrap();
    let recovered_reply = store.claim_actor_work(&sub, "actor", 401).unwrap().unwrap();
    assert_eq!(
        recovered_reply.existing_reply.unwrap().message_id,
        first.reply_message_id
    );
    assert!(!reply(&store, &first, "actor").unwrap().inserted);
    store
        .complete_actor_work(&sub, &first.work_id, "actor", ActorWorkOutcome::Replied)
        .unwrap();
    store
        .complete_actor_work(&sub, &first.work_id, "actor", ActorWorkOutcome::Replied)
        .unwrap();
    let second = store.claim_actor_work(&sub, "actor", 701).unwrap().unwrap();
    assert_eq!(second.message_id, "second");
    store
        .complete_actor_work(&sub, &second.work_id, "actor", ActorWorkOutcome::Failed)
        .unwrap();
    assert!(store
        .claim_actor_work(&sub, "actor", 1002)
        .unwrap()
        .is_none());
    assert_eq!(store.room_tip("lobby").unwrap(), 3); // no duplicate reply or self-wake
}

#[test]
fn actor_work_routing_retention_and_no_cursor_jump() {
    let store = Store::open_in_memory().unwrap();
    let addressed = subscribe(&store, "addressed", WakeMode::Addressed);
    let listen = subscribe(&store, "listener", WakeMode::Listen);
    send(&store, "unaddressed", "human", &[]);
    send(&store, "addressed1", "human", &["addressed".into()]);
    send(&store, "addressed2", "human", &["addressed".into()]);
    assert!(store
        .claim_actor_work(&listen, "listener", 0)
        .unwrap()
        .is_none());
    let first = store
        .claim_actor_work(&addressed, "addressed", 0)
        .unwrap()
        .unwrap();
    assert_eq!(first.message_id, "addressed1");
    let later = store
        .load_due_deliveries(Utc::now() + chrono::Duration::days(100), 10)
        .unwrap();
    assert_eq!(later.len(), 1); // only first pending, and no 24h expiry
    assert_eq!(store.purge_messages_by_tier("free", "+1 days").unwrap(), 0);
    let second_id: String = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT delivery_id FROM subscription_deliveries WHERE message_id = 'addressed2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(store
        .complete_actor_work(
            &addressed,
            &second_id,
            "addressed",
            ActorWorkOutcome::Skipped
        )
        .is_err());
    store
        .complete_actor_work(
            &addressed,
            &first.work_id,
            "addressed",
            ActorWorkOutcome::Skipped,
        )
        .unwrap();
    let next = store
        .claim_actor_work(&addressed, "addressed", 301)
        .unwrap()
        .unwrap();
    assert_eq!(next.message_id, "addressed2");
    store.delete_subscription(&addressed, "owner").unwrap();
    assert!(store
        .claim_actor_work(&addressed, "addressed", 602)
        .unwrap()
        .is_none());
}
