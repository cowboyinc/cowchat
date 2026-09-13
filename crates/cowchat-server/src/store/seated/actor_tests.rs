use super::*;
use ciborium::value::Value;
use cowchat_crypto::certificates;
use k256::ecdsa::SigningKey;
use sha3::Keccak256;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn replace(raw: &[u8], updates: Vec<(&str, Value)>) -> Vec<u8> {
    let mut fields: std::collections::BTreeMap<String, Value> = ciborium::from_reader(raw).unwrap();
    for (k, v) in updates {
        fields.insert(k.into(), v);
    }
    encode(&fields)
}
fn wallet_sign(kind: u32, raw: &[u8], key: u8) -> Vec<u8> {
    let key = SigningKey::from_slice(&[key; 32]).unwrap();
    let (sig, recovery) = key
        .sign_prehash_recoverable(&Keccak256::digest(
            certificates::signing_bytes(kind, raw).unwrap(),
        ))
        .unwrap();
    let mut bytes = sig.to_bytes().to_vec();
    bytes.push(recovery.to_byte());
    bytes
}

#[tokio::test]
async fn actor_http_enrollment_verifies_fetched_control_installs_atomically_and_limits_history() {
    let mut state = crate::web::tests::test_state();
    create_empty_owned_room(&state.store);
    let now = Utc::now().timestamp_millis();
    let (owner, owner_seat, owner_cert) =
        owner_enrollment_on_chain(ROOM, "owner", now as u64 + 60_000, 42);
    state
        .store
        .enroll_seated_owner(ROOM, "master", &owner, now)
        .unwrap();
    let mut old_header = crate::seated::SealedRecord::parse(&sealed(b"old"))
        .unwrap()
        .header;
    old_header.seat = owner_seat;
    old_header.cert = owner_cert;
    old_header.gen = 0;
    old_header.chain_id = 42;
    old_header.mentions = vec!["0x0000000000000000000000000000000000000009".into()];
    let old = seal_header(old_header, b"before the actor joined");
    append(&state.store, &old, 100, now).unwrap();
    // Simulate an existing room-key rotation; rotation API is outside this test.
    state
        .store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET key_generation=1", [])
        .unwrap();
    let owner_input: serde_json::Value = serde_json::from_slice(&owner).unwrap();
    let actor_seed = [21; 32];
    let public = ed25519_dalek::SigningKey::from_bytes(&actor_seed)
        .verifying_key()
        .to_bytes();
    let seat = "0x0000000000000000000000000000000000000009";
    let identity = replace(
        &B64.decode(owner_input["identity"].as_str().unwrap())
            .unwrap(),
        vec![
            ("address", text(seat)),
            ("pubkey", Value::Bytes(public.to_vec())),
            ("role", text("actor")),
            ("gen", 3u64.into()),
            ("expires_at", Value::Null),
        ],
    );
    let member = replace(
        &B64.decode(owner_input["membership"].as_str().unwrap())
            .unwrap(),
        vec![
            ("seat", text(seat)),
            ("rights", Value::Array(vec![text("read"), text("write")])),
            ("from_gen", 1u64.into()),
        ],
    );
    let id = certificates::certificate_id(certificates::IDENTITY, &identity).unwrap();
    let key = SigningKey::from_slice(&[9; 32]).unwrap();
    let controller =
        Keccak256::digest(&key.verifying_key().to_encoded_point(false).as_bytes()[1..])[12..]
            .to_vec();
    let control = encode(&Value::Map(vec![
        (text("controller"), Value::Bytes(controller)),
        (text("certificate_commitment"), Value::Bytes(id.clone())),
        (text("authorization_generation"), 3u64.into()),
    ]));
    let (checkpoint, proof, _) = crate::actor_proof::tests::proof(now as u64, control);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let courier = axum::Router::new().route(
        "/proof/finalized-state",
        axum::routing::post(move || {
            let bytes = proof.clone();
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                bytes
            }
        }),
    );
    let courier_task = tokio::spawn(async move { axum::serve(listener, courier).await.unwrap() });
    let proof_endpoint = format!("http://{addr}/proof/finalized-state");
    let authority = Arc::new(crate::actor_proof::tests::authority(
        checkpoint,
        format!("http://{addr}/proof/finalized-state"),
    ));
    state.actor_proof_authority = Some(authority.clone());
    let store = state.store.clone();
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let enrollment=serde_json::to_vec(&serde_json::json!({"identity":B64.encode(&identity),"identity_signature":B64.encode(wallet_sign(certificates::IDENTITY,&identity,9)),
        "membership":B64.encode(&member),"membership_signature":B64.encode(wallet_sign(certificates::MEMBERSHIP,&member,7))})).unwrap();
    let target = format!("/rooms/{ROOM}/actors");
    let http = reqwest::Client::new();
    let signed = |method: &str, target: &str, body: &[u8], nonce: u8| {
        let projection = encode(&Value::Array(vec![
            text(method),
            text(target),
            Value::Bytes(Sha256::digest(body).to_vec()),
            (now as u64).into(),
            Value::Bytes(vec![nonce; 16]),
        ]));
        let signature = request::sign(&projection, &actor_seed).unwrap();
        (projection, signature)
    };
    let send = |body: Vec<u8>, nonce| {
        let (p, s) = signed("POST", &target, &body, nonce);
        http.post(format!("http://{addr}{target}"))
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(body)
    };
    let mut invalid: serde_json::Value = serde_json::from_slice(&enrollment).unwrap();
    invalid["membership_signature"] = B64
        .encode(wallet_sign(certificates::MEMBERSHIP, &member, 9))
        .into();
    assert_eq!(
        send(serde_json::to_vec(&invalid).unwrap(), 101)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_actor BEFORE INSERT ON seated_credentials WHEN NEW.actor_chain_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        send(enrollment.clone(), 102).send().await.unwrap().status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    let floor_count: i64 = store
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM seated_actor_floors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(floor_count, 0);
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_actor;")
        .unwrap();
    let response = send(enrollment.clone(), 102).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["seat"],
        seat
    );
    assert_eq!(
        send(enrollment.clone(), 102).send().await.unwrap().status(),
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(
        send(enrollment.clone(), 103).send().await.unwrap().status(),
        reqwest::StatusCode::OK
    );
    let cert = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let subscription=serde_json::to_vec(&serde_json::json!({"subscription_id":uuid::Uuid::new_v4().to_string(),"transport_generation":0,
        "webhook_url":proof_endpoint,"secret":"0123456789abcdef0123456789abcdef","after":0})).unwrap();
    let sub_target = format!("/rooms/{ROOM}/subscriptions");
    let (p, s) = signed("POST", &sub_target, &subscription, 108);
    assert_eq!(
        http.post(format!("http://{addr}{sub_target}"))
            .header("x-cowchat-certificate", &cert)
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(subscription)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    // The signed mention predates the granted key generation and cannot wake it.
    assert!(store
        .load_due_deliveries(Utc::now(), 10)
        .unwrap()
        .is_empty());
    let mut header = crate::seated::SealedRecord::parse(&old).unwrap().header;
    header.message_id = uuid::Uuid::new_v4().to_string();
    header.seat = seat.into();
    header.role = "actor".into();
    header.cert = cert.clone();
    header.gen = 1;
    let sealed = envelope::seal(
        &encode(&header),
        &[42; 32],
        b"authenticated actor reply",
        &actor_seed,
    )
    .unwrap();
    let Value::Array(parts) = ciborium::from_reader(sealed.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(header), Value::Text(ciphertext), Value::Bytes(sig)] = parts.as_slice()
    else {
        panic!()
    };
    let header: crate::seated::RecordHeader = ciborium::from_reader(header.as_slice()).unwrap();
    let mut record = serde_json::to_value(header).unwrap();
    record["body"] = ciphertext.clone().into();
    record["sig"] = B64.encode(sig).into();
    let bytes = serde_json::to_vec(&record).unwrap();
    let (p, s) = signed("POST", TARGET, &bytes, 104);
    assert_eq!(
        http.post(format!("http://{addr}{TARGET}"))
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(bytes)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    let history = format!("{TARGET}?transport_generation=0&after=0&limit=100");
    let (p, s) = signed("GET", &history, b"", 105);
    let page: serde_json::Value = http
        .get(format!("http://{addr}{history}"))
        .header("x-cowchat-certificate", &cert)
        .header("x-cowchat-request", B64.encode(p))
        .header("x-cowchat-signature", B64.encode(s))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
    assert_eq!(page["cursor"]["position"], 2);
    assert_eq!(page["records"][0]["record"], record);
    let due = store.load_due_deliveries(Utc::now(), 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].message_seq, 2);
    let parsed = crate::seated::SealedRecord::parse(&serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(
        envelope::open(
            &parsed.header_cbor,
            &parsed.body,
            &public,
            &parsed.signature,
            &[42; 32]
        )
        .unwrap(),
        b"authenticated actor reply"
    );
    // A persisted newer floor must survive a stale enrollment fetch.
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_actor_floors SET height=11", [])
        .unwrap();
    assert_eq!(
        send(enrollment.clone(), 106).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_actor_floors SET height=10", [])
        .unwrap();
    let (p, s) = signed("POST", &target, &enrollment, 109);
    let prepared = store
        .prepare_actor_enrollment(ROOM, &target, &enrollment, &p, &s, now)
        .unwrap();
    let verified = authority.fetch(prepared.actor()).await.unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation=1", [])
        .unwrap();
    assert!(matches!(
        store.install_actor_enrollment(prepared, verified, now),
        Err(StoreError::SeatedAuthorization)
    ));
    let consumed: i64 = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM seated_request_nonces WHERE public_key=?1 AND nonce=?2",
            params![public.as_slice(), vec![109u8; 16]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(consumed, 0);
    server.abort();
    courier_task.abort();
}
