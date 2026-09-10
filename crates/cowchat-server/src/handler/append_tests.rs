use super::*;
use dashmap::DashMap;

#[tokio::test]
async fn append_handler_retry_has_original_response_without_broadcast_mentions_turn_or_quota_effects(
) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let broker = Arc::new(Broker::new(
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
    ));
    let (sender, mut received) = tokio::sync::mpsc::channel(32);
    broker.agents.insert(
        "peer".into(),
        crate::connection::AgentConnection::new(
            serde_json::from_value(serde_json::json!({"agent_id":"peer", "name":"Peer"})).unwrap(),
            "session".into(),
            sender,
            tokio::spawn(async {}),
            tokio::spawn(async {}),
            Arc::new(tokio::sync::Notify::new()),
            "key".into(),
        ),
    );
    broker.join_room("sender", "lobby", || true).unwrap();
    broker.join_room("peer", "lobby", || true).unwrap();
    let limiter = Arc::new(RateLimiter::new());
    let webhooks = Arc::new(crate::webhooks::WebhookManager::new(store.clone(), true));
    let payload = serde_json::json!({"room_id":"lobby", "message_id":"stable", "content":"hello", "mentions":["peer"]});
    let first = handle_send_message(
        Some("1"),
        payload.clone(),
        "sender",
        "Sender",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(first.frame_type, FrameType::Ok);
    assert_eq!(first.payload["message_id"], "stable");
    let mut first_events = Vec::new();
    while let Ok(event) = received.try_recv() {
        first_events.push(event.frame_type);
    }
    assert!(first_events.contains(&FrameType::MessageReceived));
    assert!(first_events.contains(&FrameType::Mention));
    // Set the turn back to sender so an accidental retry advance is observable.
    broker.advance_turn_from("lobby", "peer");
    let retry = handle_send_message(
        Some("2"),
        payload.clone(),
        "sender",
        "Renamed sender",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(retry.payload, first.payload);
    assert!(received.try_recv().is_err());
    assert_eq!(broker.turn_holder("lobby").as_deref(), Some("sender"));
    let mut two_messages = TierLimits::for_tier("free");
    two_messages.max_messages_per_minute = 2;
    assert!(limiter.check_message_rate("key", &two_messages));
    let limits = TierLimits::for_tier("free");
    for _ in 1..limits.max_messages_per_minute {
        limiter.increment_message("key");
    }
    assert!(!limiter.check_message_rate("key", &limits));
    let full_retry = handle_send_message(
        Some("3"),
        payload.clone(),
        "sender",
        "Sender",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(full_retry.payload, first.payload);
    let mut conflict = payload.clone();
    conflict["content"] = serde_json::json!("changed");
    let response = handle_send_message(
        Some("4"),
        conflict,
        "sender",
        "Sender",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(response.payload["code"], "message_conflict");
    let other_sender = handle_send_message(
        Some("5"),
        payload.clone(),
        "peer",
        "Peer",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(other_sender.payload["code"], "message_conflict");
    let unjoined = handle_send_message(
        Some("6"),
        payload.clone(),
        "outsider",
        "Outside",
        &broker,
        &store,
        "key",
        &limiter,
        false,
        &webhooks,
    )
    .await;
    assert_eq!(unjoined.payload["code"], "not_in_room");
    assert_eq!(store.room_tip("lobby").unwrap(), 1);
    assert!(received.try_recv().is_err());
    // Legacy omitted-ID requests still mint a new ID on each distinct send.
    let mut legacy = payload;
    legacy.as_object_mut().unwrap().remove("message_id");
    let a = handle_send_message(
        None,
        legacy.clone(),
        "sender",
        "Sender",
        &broker,
        &store,
        "",
        &limiter,
        true,
        &webhooks,
    )
    .await;
    let b = handle_send_message(
        None, legacy, "sender", "Sender", &broker, &store, "", &limiter, true, &webhooks,
    )
    .await;
    assert_ne!(a.payload["message_id"], b.payload["message_id"]);
    assert_eq!(store.room_tip("lobby").unwrap(), 3);
}
