use super::*;
use ciborium::value::Value;
use cowchat_crypto::{canonical, envelope};
use sha2::{Digest, Sha256};

const ROOM: &str = "10000000-0000-4000-8000-000000000001";
const ID: &str = "20000000-0000-4000-8000-000000000001";
const SEAT: &str = "0x1111111111111111111111111111111111111111";
const TARGET: &str = "/rooms/10000000-0000-4000-8000-000000000001/messages";
fn seed() -> Vec<u8> {
    (0..32).collect()
}
fn encode(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).unwrap();
    canonical::canonicalize(&bytes).unwrap()
}
fn text(value: &str) -> Value {
    Value::Text(value.into())
}

// This fixture supplies already-authenticated provisioning state. It is NOT
// a credential-enrollment API or proof of an actor controller on a live network.
fn install_fixture(store: &Store) {
    store
        .create_room(ROOM, "seated-test", None, None, Some("fixture"))
        .unwrap();
    store
        .create_subscription_with_mention(
            "wake",
            ROOM,
            "fixture",
            "https://example.com/wake",
            "test-secret",
            &[],
            None,
            None,
            true,
            0,
            Some(SEAT),
        )
        .unwrap();
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../../fixtures/v3/envelopes.json")).unwrap();
    let public_hex = fixtures[0]["public_key_hex"].as_str().unwrap();
    let public: Vec<u8> = (0..public_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&public_hex[i..i + 2], 16).unwrap())
        .collect();
    let context = Value::Map(vec![
        (text("chain_id"), 1u64.into()),
        (text("room"), text(ROOM)),
        (text("gen"), 2u64.into()),
        (text("seat"), text(SEAT)),
        (text("role"), text("owner")),
        (text("cert"), text("fixture-cert")),
        (text("public_key"), Value::Bytes(public.clone())),
        (
            text("rights"),
            Value::Array(vec![text("read"), text("write")]),
        ),
        (
            text("expires_at"),
            (Utc::now().timestamp_millis() as u64 + 60_000).into(),
        ),
        (text("door_kind"), Value::Null),
        (text("bound_sender"), Value::Null),
        (text("forwarded_seat"), Value::Null),
    ]);
    let conn = store.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO seated_rooms(room_id, auth_generation, key_generation) VALUES (?1, 1, 2)",
        [ROOM],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO seated_credentials(room_id, cert_id, seat, display_name, auth_generation, public_key, trusted_context) VALUES (?1, 'fixture-cert', ?2, 'Owner', 1, ?3, ?4)",
        params![ROOM, SEAT, public, encode(&context)],
    )
    .unwrap();
}
fn sealed(plaintext: &[u8]) -> Vec<u8> {
    let header = crate::seated::RecordHeader {
        v: 3,
        message_id: ID.into(),
        chain_id: 1,
        room: ROOM.into(),
        seat: SEAT.into(),
        role: "owner".into(),
        via: Some("dashboard".into()),
        via_sender: None,
        class: "message".into(),
        reply_to: None,
        mentions: vec![SEAT.into()],
        wake_hint: "normal".into(),
        gen: 2,
        cert: "fixture-cert".into(),
        nonce: B64.encode([0; 12]),
    };
    seal_header(header, plaintext)
}
fn seal_header(header: crate::seated::RecordHeader, plaintext: &[u8]) -> Vec<u8> {
    let sealed = envelope::seal(&encode(&header), &[42; 32], plaintext, &seed()).unwrap();
    let Value::Array(parts) = ciborium::from_reader(sealed.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(header), Value::Text(body), Value::Bytes(sig)] = parts.as_slice() else {
        panic!()
    };
    let header: crate::seated::RecordHeader = ciborium::from_reader(header.as_slice()).unwrap();
    let mut wire = serde_json::to_value(header).unwrap();
    wire["body"] = body.clone().into();
    wire["sig"] = B64.encode(sig).into();
    serde_json::to_vec(&wire).unwrap()
}
fn signed(body: &[u8], nonce: u8, now: i64) -> (Vec<u8>, Vec<u8>) {
    let projection = encode(&Value::Array(vec![
        text("POST"),
        text(TARGET),
        Value::Bytes(Sha256::digest(body).to_vec()),
        (now as u64).into(),
        Value::Bytes(vec![nonce; 16]),
    ]));
    let sig = request::sign(&projection, &seed()).unwrap();
    (projection, sig)
}
fn append(store: &Store, body: &[u8], nonce: u8, now: i64) -> Result<AppendResult, StoreError> {
    let (projection, signature) = signed(body, nonce, now);
    store.append_seated_record(ROOM, "POST", TARGET, body, &projection, &signature, now)
}

#[test]
fn seated_append_auth_nonce_idempotency_and_revocation_share_one_transaction() {
    let store = Store::open_in_memory().unwrap();
    install_fixture(&store);
    let now = Utc::now().timestamp_millis();
    let body = sealed(b"private request");
    assert!(append(&store, &body, 1, now).unwrap().inserted);
    assert!(matches!(
        append(&store, &body, 1, now),
        Err(StoreError::SeatedReplay)
    ));
    let retry = append(&store, &body, 2, now).unwrap();
    assert!(!retry.inserted);
    assert_eq!(retry.message.seq, 1);
    assert!(retry.message.content.starts_with("cow1:"));
    assert!(!serde_json::to_string(&retry.message)
        .unwrap()
        .contains("private request"));
    assert!(matches!(
        append(&store, &sealed(b"different candidate"), 3, now),
        Err(StoreError::MessageConflict)
    ));
    // Failed append rolled back nonce consumption as well as the message/outbox.
    assert!(!append(&store, &body, 3, now).unwrap().inserted);
    assert_eq!(store.load_due_deliveries(Utc::now(), 32).unwrap().len(), 1);
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation = 2", [])
        .unwrap();
    assert!(matches!(
        append(&store, &body, 4, now),
        Err(StoreError::SeatedAuthorization)
    ));
    assert_eq!(store.room_tip(ROOM).unwrap(), 1);
    assert!(matches!(
        store.get_history(ROOM, 10, None),
        Err(StoreError::SeatedRequestRequired)
    ));
    assert!(matches!(
        store.insert_message(
            "legacy",
            ROOM,
            SEAT,
            "Spoofed",
            "plaintext",
            None,
            &serde_json::json!({})
        ),
        Err(StoreError::SeatedRequestRequired)
    ));
    assert_eq!(store.purge_messages_by_tier("free", "+1 hour").unwrap(), 0);
}

#[tokio::test]
async fn seated_http_append_binds_actual_request_and_returns_only_ciphertext_receipt() {
    let state = crate::web::tests::test_state();
    install_fixture(&state.store);
    let store = state.store.clone();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let body = sealed(b"private HTTP request");
    let (projection, signature) = signed(&body, 20, Utc::now().timestamp_millis());
    let http = reqwest::Client::new();
    let response = http
        .post(format!("http://{addr}{TARGET}"))
        .header("x-cowchat-request", B64.encode(&projection))
        .header("x-cowchat-signature", B64.encode(&signature))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        receipt,
        serde_json::json!({"message_id":ID,"status":"accepted","seq":1})
    );
    let changed = sealed(b"changed body");
    let response = http
        .post(format!("http://{addr}{TARGET}"))
        .header("x-cowchat-request", B64.encode(&projection))
        .header("x-cowchat-signature", B64.encode(&signature))
        .body(changed)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response = http
        .post(format!("http://{addr}{TARGET}"))
        .header("x-cowchat-key", "master")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(store.room_tip(ROOM).unwrap(), 1);
    server.abort();
}

fn owner_enrollment(room: &str, role: &str, expiry: u64) -> (Vec<u8>, String, String) {
    use cowchat_crypto::certificates;
    use k256::ecdsa::SigningKey;
    use sha3::Keccak256;
    let wallet = SigningKey::from_slice(&[7; 32]).unwrap();
    let point = wallet.verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    let seat = format!(
        "0x{}",
        hash[12..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../../fixtures/v3/envelopes.json")).unwrap();
    let key = fixtures[0]["public_key_hex"].as_str().unwrap();
    let public: Vec<u8> = (0..key.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&key[i..i + 2], 16).unwrap())
        .collect();
    let identity = encode(&Value::Map(vec![
        (text("v"), 3u64.into()),
        (text("chain_id"), 1u64.into()),
        (text("address"), text(&seat)),
        (text("pubkey"), Value::Bytes(public)),
        (text("enc_pubkey"), Value::Bytes(vec![8; 32])),
        (text("role"), text(role)),
        (text("aud"), text("cowchat")),
        (text("gen"), 0u64.into()),
        (
            text("expires_at"),
            if role == "actor" {
                Value::Null
            } else {
                expiry.into()
            },
        ),
    ]));
    let membership = encode(&Value::Map(vec![
        (text("v"), 3u64.into()),
        (text("chain_id"), 1u64.into()),
        (text("room"), text(room)),
        (text("seat"), text(&seat)),
        (
            text("rights"),
            Value::Array(["manage", "read", "write"].into_iter().map(text).collect()),
        ),
        (text("door_kind"), Value::Null),
        (text("bound_sender"), Value::Null),
        (text("from_gen"), 0u64.into()),
        (text("gen"), 0u64.into()),
        (text("signer_kind"), text("wallet")),
        (text("signer_key"), Value::Null),
        (text("expires_at"), Value::Null),
    ]));
    let sign = |kind, bytes: &[u8]| {
        let digest = Keccak256::digest(certificates::signing_bytes(kind, bytes).unwrap());
        let (signature, recovery) = wallet.sign_prehash_recoverable(&digest).unwrap();
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery.to_byte());
        B64.encode(bytes)
    };
    let cert = certificates::certificate_id(certificates::IDENTITY, &identity)
        .unwrap()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let raw = serde_json::to_vec(&serde_json::json!({"identity":B64.encode(&identity),"identity_signature":sign(certificates::IDENTITY,&identity),
        "membership":B64.encode(&membership),"membership_signature":sign(certificates::MEMBERSHIP,&membership)})).unwrap();
    (raw, seat, cert)
}
fn create_empty_owned_room(store: &Store) {
    store
        .create_room_with_visibility(
            ROOM,
            "owner-room",
            None,
            None,
            Some("human"),
            "private",
            Some("master"),
            false,
        )
        .unwrap();
}

#[test]
fn owner_enrollment_requires_wallet_proofs_and_empty_owned_room_then_retries_without_restoring_revocation(
) {
    let store = Store::open_in_memory().unwrap();
    create_empty_owned_room(&store);
    let now = Utc::now().timestamp_millis();
    let (raw, seat, cert) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    assert!(matches!(
        store.enroll_seated_owner(ROOM, "outsider", &raw, now),
        Err(StoreError::SeatedAuthorization)
    ));
    let (wrong_room, _, _) = owner_enrollment("another-room", "owner", now as u64 + 60_000);
    assert!(store
        .enroll_seated_owner(ROOM, "master", &wrong_room, now)
        .is_err());
    let (actor, _, _) = owner_enrollment(ROOM, "actor", now as u64 + 60_000);
    assert!(store
        .enroll_seated_owner(ROOM, "master", &actor, now)
        .is_err());
    let (expired, _, _) = owner_enrollment(ROOM, "owner", now as u64 - 1);
    assert!(store
        .enroll_seated_owner(ROOM, "master", &expired, now)
        .is_err());
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_owner BEFORE INSERT ON seated_credentials BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store
        .enroll_seated_owner(ROOM, "master", &raw, now)
        .is_err());
    assert!(!store.is_seated_room(ROOM).unwrap());
    assert!(!store.get_room(ROOM).unwrap().unwrap().encrypted);
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_owner")
        .unwrap();
    assert_eq!(
        store
            .enroll_seated_owner(ROOM, "master", &raw, now)
            .unwrap(),
        (seat.clone(), cert.clone())
    );
    assert_eq!(
        store
            .enroll_seated_owner(ROOM, "master", &raw, now)
            .unwrap(),
        (seat, cert)
    );
    assert!(store.get_room(ROOM).unwrap().unwrap().encrypted);
    assert!(matches!(
        store.create_subscription(
            "legacy-sub",
            ROOM,
            "master",
            "https://example.com/wake",
            "secret",
            &[],
            None,
            None,
            false,
            0
        ),
        Err(StoreError::SeatedRequestRequired)
    ));

    assert!(matches!(
        store.destroy_room_authorized(ROOM, "human", "master", true),
        Err(DestroyRoomError::AccessDenied)
    ));
    assert!(matches!(
        store.rename_room_authorized(ROOM, "human", "master", true, "replacement"),
        Err(RenameRoomError::AccessDenied)
    ));
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation = 1", [])
        .unwrap();
    assert!(matches!(
        store.enroll_seated_owner(ROOM, "master", &raw, now),
        Err(StoreError::MessageConflict)
    ));
    let other = Store::open_in_memory().unwrap();
    create_empty_owned_room(&other);
    other
        .insert_message(
            "old",
            ROOM,
            "human",
            "Human",
            "plaintext",
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    assert!(matches!(
        other.enroll_seated_owner(ROOM, "master", &raw, now),
        Err(StoreError::MessageConflict)
    ));
}

#[tokio::test]
async fn owner_enrolls_through_real_http_then_posts_signed_encrypted_record() {
    let state = crate::web::tests::test_state();
    create_empty_owned_room(&state.store);
    let store = state.store.clone();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let now = Utc::now().timestamp_millis();
    let (enrollment, seat, cert) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    let http = reqwest::Client::new();
    let owner_url = format!("http://{addr}/rooms/{ROOM}/owner");
    assert_eq!(
        http.post(&owner_url)
            .body(enrollment.clone())
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let response = http
        .post(owner_url)
        .header("x-cowchat-key", "master")
        .body(enrollment)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: serde_json::Value = response.json().await.unwrap();
    assert_eq!(receipt["seat"], seat);
    assert_eq!(receipt["cert"], cert);
    let mut header = crate::seated::SealedRecord::parse(&sealed(b"template"))
        .unwrap()
        .header;
    header.seat = seat;
    header.cert = cert;
    header.gen = 0;
    header.mentions.clear();
    let body = seal_header(header, b"only the owner can decrypt this");
    let (projection, signature) = signed(&body, 40, Utc::now().timestamp_millis());
    let response = http
        .post(format!("http://{addr}{TARGET}"))
        .header("x-cowchat-request", B64.encode(projection))
        .header("x-cowchat-signature", B64.encode(signature))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let persisted = store.get_message(ID).unwrap().unwrap();
    assert!(persisted.content.starts_with("cow1:"));
    assert!(!serde_json::to_string(&persisted)
        .unwrap()
        .contains("only the owner"));
    assert_eq!(persisted.agent_name, "Owner");
    server.abort();
}
