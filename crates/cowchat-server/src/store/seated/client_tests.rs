use super::*;
use cowchat_client::seated::{verify_wake, RoomError, RoomSeat, SeatedHttpClient};
use hmac::{Hmac, Mac};

const WEBHOOK_SECRET: &[u8] = &[75; 32];

fn wake_bytes() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "specversion":"1.0", "id":"fixture-dispatch", "source":format!("/rooms/{ROOM}"),
        "type":"cowchat.room.wake", "datacontenttype":"application/json",
        "data":{"room":ROOM,"message_id":ID,"seq":1,"tip":1,"since_seq":0,
                "dispatch_id":"fixture-dispatch","transport_generation":0}
    }))
    .unwrap()
}
fn wake_headers(body: &[u8], now: i64) -> Vec<(String, String)> {
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET).unwrap();
    mac.update(format!("fixture-dispatch.{now}.").as_bytes());
    mac.update(body);
    vec![
        ("webhook-id".into(), "fixture-dispatch".into()),
        ("webhook-timestamp".into(), now.to_string()),
        (
            "webhook-signature".into(),
            format!(
                "v1,{}",
                base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
            ),
        ),
    ]
}
fn seat() -> RoomSeat {
    RoomSeat {
        room: ROOM.into(),
        chain_id: 1,
        transport_generation: 0,
        key_generation: 2,
        seat: SEAT.into(),
        role: "owner".into(),
        certificate: "fixture-cert".into(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&seed().try_into().unwrap())
            .verifying_key()
            .to_bytes(),
    }
}

#[test]
fn receiver_rejects_untrusted_wake_before_using_pointer() {
    let now = Utc::now().timestamp();
    let body = wake_bytes();
    let headers = wake_headers(&body, now);
    let wake = verify_wake(&headers, &body, WEBHOOK_SECRET, ROOM, 0, now).unwrap();
    assert_eq!(wake.message_id(), ID);
    assert_eq!(wake.dispatch_id(), "fixture-dispatch");
    assert_eq!(wake.since_seq(), 0);
    assert_eq!(wake.tip(), 1);
    assert_eq!(wake.seq(), 1);
    assert!(verify_wake(&headers, b"{}", WEBHOOK_SECRET, ROOM, 0, now).is_err());
    assert!(verify_wake(&headers, &body, &[74; 32], ROOM, 0, now).is_err());
    assert!(verify_wake(&headers, &body, WEBHOOK_SECRET, ROOM, 0, now + 301).is_err());
    assert!(verify_wake(&headers, &body, WEBHOOK_SECRET, ID, 0, now).is_err());
    assert!(verify_wake(&headers, &body, WEBHOOK_SECRET, ROOM, 1, now).is_err());
    let mut duplicate = headers.clone();
    duplicate.push(("Webhook-Id".into(), "fixture-dispatch".into()));
    assert!(verify_wake(&duplicate, &body, WEBHOOK_SECRET, ROOM, 0, now).is_err());
    let mut changed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    changed["data"]["seq"] = 0.into();
    let changed = serde_json::to_vec(&changed).unwrap();
    assert!(verify_wake(
        &wake_headers(&changed, now),
        &changed,
        WEBHOOK_SECRET,
        ROOM,
        0,
        now
    )
    .is_err());
}

#[tokio::test]
async fn room_client_retries_exact_ciphertext_and_reconciles_authenticated_winner() {
    // Authenticated fixture credentials exercise the real HTTP sink; this test
    // does not claim runtime key provisioning or a gateway-triggered actor run.
    let state = crate::web::tests::test_state();
    install_fixture(&state.store);
    let now = Utc::now().timestamp_millis();
    append(&state.store, &sealed(b"trigger"), 81, now).unwrap();
    let store = state.store.clone();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let client = SeatedHttpClient::new(&format!("http://{addr}"), seat()).unwrap();
    let body = wake_bytes();
    let wake = verify_wake(
        &wake_headers(&body, now / 1000),
        &body,
        WEBHOOK_SECRET,
        ROOM,
        0,
        now / 1000,
    )
    .unwrap();
    let first = client
        .prepare_reply(&wake, b"first reply", &seed(), &[42; 32])
        .unwrap();
    let second = client
        .prepare_reply(
            &wake,
            b"a duplicate run chose other words",
            &seed(),
            &[42; 32],
        )
        .unwrap();
    assert_eq!(first.message_id(), second.message_id());
    let first_record = crate::seated::SealedRecord::parse(first.bytes()).unwrap();
    let second_record = crate::seated::SealedRecord::parse(second.bytes()).unwrap();
    assert_ne!(first_record.header.nonce, second_record.header.nonce);
    assert_ne!(first.bytes(), second.bytes());
    assert!(client
        .restore_reply("30000000-0000-4000-8000-000000000001", first.bytes())
        .is_err());
    let mut forged: serde_json::Value = serde_json::from_slice(first.bytes()).unwrap();
    forged["sig"] = B64.encode([0; 64]).into();
    assert!(client
        .restore_reply(ID, &serde_json::to_vec(&forged).unwrap())
        .is_err());
    // Simulate a lost acknowledgement by discarding it, recreating the client,
    // and recovering the exact sealed bytes saved before the first request.
    client.submit_reply(&first, &seed()).await.unwrap();
    drop(client);
    let client = SeatedHttpClient::new(&format!("http://{addr}"), seat()).unwrap();
    let restored = client.restore_reply(ID, first.bytes()).unwrap();
    assert_eq!(restored.bytes(), first.bytes());
    assert_eq!(
        client.submit_reply(&restored, &seed()).await.unwrap()["seq"],
        2
    );
    let reconciled = client.submit_reply(&second, &seed()).await.unwrap();
    assert_eq!(reconciled["seq"], 2);
    assert_eq!(reconciled["status"], "existing");
    let page = client.read_ciphertext_page(0, &seed()).await.unwrap();
    assert_eq!(page["records"].as_array().unwrap().len(), 2);
    let trigger = client
        .read_ciphertext_message(wake.message_id(), &seed())
        .await
        .unwrap();
    assert_eq!(trigger["records"].as_array().unwrap().len(), 1);
    assert_eq!(trigger["records"][0]["position"], wake.seq());
    assert_eq!(trigger["records"][0]["record"]["message_id"], ID);
    assert!(client
        .read_ciphertext_message("bad&id", &seed())
        .await
        .is_err());
    let record = crate::seated::SealedRecord::parse(
        &serde_json::to_vec(&page["records"][1]["record"]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        envelope::open(
            &record.header_cbor,
            &record.body,
            &seat().public_key,
            &record.signature,
            &[42; 32]
        )
        .unwrap(),
        b"first reply"
    );
    assert_eq!(
        store
            .get_message(first.message_id())
            .unwrap()
            .unwrap()
            .content,
        first_record.body
    );
    server.abort();
}

#[tokio::test]
async fn conflicting_http_response_cannot_forge_winner() {
    use axum::{routing::post, Json, Router};
    use reqwest::StatusCode;
    let body = wake_bytes();
    let now = Utc::now().timestamp();
    let wake = verify_wake(
        &wake_headers(&body, now),
        &body,
        WEBHOOK_SECRET,
        ROOM,
        0,
        now,
    )
    .unwrap();
    let local = SeatedHttpClient::new("http://127.0.0.1", seat()).unwrap();
    let reply = local
        .prepare_reply(&wake, b"reply", &seed(), &[42; 32])
        .unwrap();
    let mut forged: serde_json::Value = serde_json::from_slice(reply.bytes()).unwrap();
    forged["sig"] = B64.encode([0; 64]).into();
    let app = Router::new().route(
        TARGET,
        post(|| async { StatusCode::CONFLICT }).get(move || {
            let forged = forged.clone();
            async move { Json(serde_json::json!({"records":[{"position":2,"record":forged}]})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SeatedHttpClient::new(&format!("http://{addr}"), seat()).unwrap();
    assert!(matches!(
        client.submit_reply(&reply, &seed()).await,
        Err(RoomError::UnresolvedConflict)
    ));
    server.abort();
}
