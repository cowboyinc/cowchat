use super::key_envelope_tests::replace;
use super::*;
const DOOR_A: &str = "0x2222222222222222222222222222222222222222#door:slack";
const DOOR_B: &str = "0x3333333333333333333333333333333333333333#door:slack";

fn install_door(store: &Store, seat: &str, cert: &str, key: u8) {
    // Explicit authenticated-context fixture. Production door identity issuance
    // and destination binding remain separate work, not claims made by this test.
    let mut context: Vec<u8> = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT trusted_context FROM seated_credentials WHERE cert_id='fixture-cert'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let public = ed25519_dalek::SigningKey::from_bytes(&[key; 32])
        .verifying_key()
        .to_bytes();
    for (k, v) in [
        ("role", text("door")),
        ("seat", text(seat)),
        ("cert", text(cert)),
        ("public_key", Value::Bytes(public.to_vec())),
        ("door_kind", text("slack")),
        ("bound_sender", text("owner-external-id")),
        ("forwarded_seat", text(SEAT)),
    ] {
        context = replace(&context, k, v);
    }
    store.conn.lock().unwrap().execute("INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context) VALUES (?1,?2,?3,'Door',1,?4,?5)",params![ROOM,cert,seat,public.to_vec(),context]).unwrap();
}
fn input(id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"subscription_id":id,"transport_generation":0,"webhook_url":"https://example.com/wake","secret":"0123456789abcdef0123456789abcdef","after":0})).unwrap()
}
fn subscribe(store: &Store, cert: &str, id: &str, key: u8, nonce: u8, now: i64) {
    let body = input(id);
    let target = format!("/rooms/{ROOM}/subscriptions");
    let (p, _) = signed_for("POST", &target, &body, nonce, now);
    let s = request::sign(&p, &[key; 32]).unwrap();
    store
        .subscribe_seated(ROOM, cert, "POST", &target, &body, &p, &s, now)
        .unwrap();
}
fn post(
    store: &Store,
    class: &str,
    hint: &str,
    door: Option<(&str, &str, u8, bool)>,
    nonce: u8,
    now: i64,
) -> String {
    let mut h = crate::seated::SealedRecord::parse(&sealed(b"template"))
        .unwrap()
        .header;
    h.message_id = uuid::Uuid::new_v4().to_string();
    h.class = class.into();
    h.wake_hint = hint.into();
    h.mentions = vec![];
    let signing_seed = if let Some((seat, cert, key, forward)) = door {
        h.cert = cert.into();
        h.role = if forward { "owner" } else { "door" }.into();
        h.seat = if forward { SEAT } else { seat }.into();
        h.via = if forward { Some("slack".into()) } else { None };
        h.via_sender = if forward {
            Some("owner-external-id".into())
        } else {
            None
        };
        vec![key; 32]
    } else {
        seed()
    };
    let id = h.message_id.clone();
    let sealed = envelope::seal(
        &encode(&h),
        &[42; 32],
        b"private door content",
        &signing_seed,
    )
    .unwrap();
    let Value::Array(parts) = ciborium::from_reader(sealed.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(header), Value::Text(body), Value::Bytes(sig)] = parts.as_slice() else {
        panic!()
    };
    let header: crate::seated::RecordHeader = ciborium::from_reader(header.as_slice()).unwrap();
    let mut record = serde_json::to_value(header).unwrap();
    record["body"] = body.clone().into();
    record["sig"] = B64.encode(sig).into();
    let raw = serde_json::to_vec(&record).unwrap();
    let (p, _) = signed_for("POST", TARGET, &raw, nonce, now);
    let s = request::sign(&p, &signing_seed).unwrap();
    store
        .append_seated_record(ROOM, "POST", TARGET, &raw, &p, &s, now)
        .unwrap();
    id
}
fn queued(store: &Store, subscription: &str) -> Vec<String> {
    let conn = store.conn.lock().unwrap();
    let mut stmt=conn.prepare("SELECT message_id FROM subscription_deliveries WHERE subscription_id=?1 AND status='pending' ORDER BY message_seq").unwrap();
    stmt.query_map([subscription], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn door_filters_live_backfill_restart_and_same_provider_echo_suppression() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doors.sqlite");
    let store = Store::open(&path).unwrap();
    install_fixture(&store);
    install_door(&store, DOOR_A, "door-a", 17);
    install_door(&store, DOOR_B, "door-b", 18);
    let now = Utc::now().timestamp_millis();
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    subscribe(&store, "door-a", &a, 17, 1, now);
    let ordinary = post(&store, "message", "normal", None, 2, now);
    let quiet = post(&store, "message", "none", None, 3, now);
    post(&store, "thinking", "none", None, 4, now);
    // The current envelope codec rejects data records before routing.
    let mut unsupported = crate::seated::SealedRecord::parse(&sealed(b"template"))
        .unwrap()
        .header;
    unsupported.class = "data".into();
    assert!(envelope::validate_header(&encode(&unsupported)).is_err());
    let inbound_a = post(
        &store,
        "message",
        "normal",
        Some((DOOR_A, "door-a", 17, true)),
        6,
        now,
    );
    let own_a = post(
        &store,
        "message",
        "normal",
        Some((DOOR_A, "door-a", 17, false)),
        7,
        now,
    );
    let inbound_b = post(
        &store,
        "message",
        "none",
        Some((DOOR_B, "door-b", 18, true)),
        8,
        now,
    );
    assert_eq!(
        queued(&store, &a),
        vec![ordinary.clone(), quiet.clone(), inbound_b.clone()]
    );
    // Same provider, different destination seat: do not suppress the other door.
    subscribe(&store, "door-b", &b, 18, 9, now);
    assert_eq!(
        queued(&store, &b),
        vec![
            ordinary.clone(),
            quiet.clone(),
            inbound_a.clone(),
            own_a.clone()
        ]
    );
    // A future authenticated system writer must supply authenticated routing
    // metadata. This fixture does not expose a new system append authority.
    let mut h = crate::seated::SealedRecord::parse(&sealed(b"fixture"))
        .unwrap()
        .header;
    h.message_id = uuid::Uuid::new_v4().to_string();
    h.class = "system".into();
    h.wake_hint = "none".into();
    h.mentions = vec![];
    let id = h.message_id.clone();
    let metadata = serde_json::json!({"v":3,"type":"system","wake_hint":"none","signer_seat":"room-service","header_cbor":B64.encode(encode(&h))});
    {
        let mut conn = store.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        Store::append_on(
            &tx,
            &MessageAppend {
                message_id: &id,
                room_id: ROOM,
                agent_id: "room-service",
                agent_name: "System",
                content: "opaque system fixture",
                reply_to: None,
                metadata: &metadata,
                mentions: &[],
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        queued(&store, &a),
        vec![ordinary.clone(), quiet.clone(), inbound_b, id.clone()]
    );
    assert_eq!(
        queued(&store, &b),
        vec![ordinary, quiet, inbound_a, own_a, id]
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    for delivery in store
        .load_due_deliveries(Utc::now(), 100)
        .unwrap()
        .into_iter()
        .filter(|d| d.subscription_id == a || d.subscription_id == b)
    {
        let payload = store
            .seated_wake_payload(&delivery.subscription_id, &delivery.delivery_id, now)
            .unwrap()
            .unwrap();
        assert!(!payload.contains("private door content"));
    }
    assert_eq!(queued(&store, &a).len(), 4);
    assert_eq!(queued(&store, &b).len(), 5);
}

#[tokio::test]
async fn door_subscription_http_uses_verified_role_and_repair_enforces_current_floor() {
    let state = crate::web::tests::test_state();
    let store = state.store.clone();
    install_fixture(&store);
    install_door(&store, DOOR_A, "door-a", 17);
    let now = Utc::now().timestamp_millis();
    let id = uuid::Uuid::new_v4().to_string();
    let body = input(&id);
    let target = format!("/rooms/{ROOM}/subscriptions");
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let http = reqwest::Client::new();
    let send = |target: &str, body: Vec<u8>, nonce| {
        let (p, _) = signed_for("POST", target, &body, nonce, now);
        let s = request::sign(&p, &[17; 32]).unwrap();
        http.post(format!("http://{addr}{target}"))
            .header("x-cowchat-certificate", "door-a")
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(body)
            .send()
    };
    assert_eq!(send(&target, body, 20).await.unwrap().status(), 200);
    let only_mention: Option<String> = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT only_mention FROM subscriptions WHERE subscription_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(only_mention.is_none());
    let message = post(&store, "message", "none", None, 21, now);
    assert_eq!(queued(&store, &id), vec![message]);
    let delivery = store
        .load_due_deliveries(Utc::now(), 20)
        .unwrap()
        .into_iter()
        .find(|d| d.subscription_id == id)
        .unwrap();
    let original = store
        .seated_wake_payload(&id, &delivery.delivery_id, now)
        .unwrap()
        .unwrap();
    store.conn.lock().unwrap().execute_batch("UPDATE seated_rooms SET key_generation=3; UPDATE seated_credentials SET from_key_generation=3 WHERE cert_id='door-a';").unwrap();
    assert!(store
        .seated_wake_payload(&id, &delivery.delivery_id, now)
        .is_err());
    let repair=serde_json::to_vec(&serde_json::json!({"operation_id":uuid::Uuid::new_v4().to_string(),"transport_generation":0,"expected_revision":0,"action":{"kind":"repair"}})).unwrap();
    assert_eq!(
        send(
            &format!("/rooms/{ROOM}/subscriptions/{id}/lifecycle"),
            repair,
            22
        )
        .await
        .unwrap()
        .status(),
        200
    );
    assert!(queued(&store, &id).is_empty());
    let retained: String = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT payload FROM seated_wakes WHERE delivery_id=?1",
            [&delivery.delivery_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, original);
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM seated_credentials WHERE cert_id='door-a'", [])
        .unwrap();
    assert!(store
        .seated_wake_payload(&id, &delivery.delivery_id, now)
        .is_err());
    server.abort();
}
