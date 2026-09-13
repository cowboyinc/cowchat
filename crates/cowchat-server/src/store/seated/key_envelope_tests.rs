use super::*;
use cowchat_crypto::{certificates, key_envelope, keys};

fn hpke(name: &str) -> Vec<u8> {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../fixtures/v3/hpke.json")).unwrap();
    let h = v[0][name].as_str().unwrap();
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect()
}
fn replace(raw: &[u8], name: &str, value: Value) -> Vec<u8> {
    let mut map: std::collections::BTreeMap<String, Value> = ciborium::from_reader(raw).unwrap();
    map.insert(name.into(), value);
    encode(&map)
}
fn setup(store: &Store, now: i64) -> (String, String) {
    create_empty_owned_room(store);
    let (raw, seat, _) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    let mut input: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let identity = replace(
        &B64.decode(input["identity"].as_str().unwrap()).unwrap(),
        "enc_pubkey",
        Value::Bytes(hpke("recipient_public_hex")),
    );
    let key = k256::ecdsa::SigningKey::from_slice(&[7; 32]).unwrap();
    let (sig, recovery) = key
        .sign_prehash_recoverable(&sha3::Keccak256::digest(
            certificates::signing_bytes(certificates::IDENTITY, &identity).unwrap(),
        ))
        .unwrap();
    let mut sig = sig.to_bytes().to_vec();
    sig.push(recovery.to_byte());
    input["identity"] = B64.encode(&identity).into();
    input["identity_signature"] = B64.encode(sig).into();
    let (_, cert) = store
        .enroll_seated_owner(ROOM, "master", &serde_json::to_vec(&input).unwrap(), now)
        .unwrap();
    (seat, cert)
}
fn make_scope(seat: &str, cert: &str) -> Vec<u8> {
    encode(&Value::Map(vec![
        (text("v"), 1u64.into()),
        (text("chain_id"), 1u64.into()),
        (text("room"), text(ROOM)),
        (text("auth_generation"), 0u64.into()),
        (text("key_generation"), 0u64.into()),
        (text("transport_generation"), 0u64.into()),
        (text("recipient_seat"), text(seat)),
        (text("recipient_cert"), text(cert)),
        (
            text("recipient_key"),
            Value::Bytes(hpke("recipient_public_hex")),
        ),
        (text("publisher_cert"), text(cert)),
        (text("purpose"), text("room-generation-secret")),
    ]))
}
fn wire(scope: &[u8], wrapped: &[u8], cert: &str, transport: u64) -> Vec<u8> {
    serde_json::to_vec(&crate::seated::KeyEnvelope {
        publisher_cert: cert.into(),
        transport_generation: transport,
        scope: B64.encode(scope),
        wrapped: B64.encode(wrapped),
        signature: B64.encode(key_envelope::sign(scope, wrapped, &seed()).unwrap()),
    })
    .unwrap()
}
fn call(
    store: &Store,
    cert: &str,
    method: &str,
    target: &str,
    body: &[u8],
    nonce: u8,
    now: i64,
) -> Result<serde_json::Value, StoreError> {
    let (projection, signature) = signed_for(method, target, body, nonce, now);
    store.seated_key_envelope(
        ROOM,
        cert,
        "0",
        cert,
        method,
        target,
        body,
        &projection,
        &signature,
        now,
    )
}

#[tokio::test]
async fn key_envelope_http_roundtrip_uses_real_enrollment_hpke_and_current_authority() {
    let state = crate::web::tests::test_state();
    let store = state.store.clone();
    let now = Utc::now().timestamp_millis();
    let (seat, cert) = setup(&store, now);
    let scope = make_scope(&seat, &cert);
    let secret = hpke("generation_secret_hex");
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &secret).unwrap();
    let body = wire(&scope, &wrapped, &cert, 0);
    let path = format!("/rooms/{ROOM}/key-envelopes/{cert}/0");
    let get = format!("{path}?transport_generation=0");
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let http = reqwest::Client::new();
    let send = |method: &str, target: &str, body: Vec<u8>, nonce| {
        let (p, s) = signed_for(method, target, &body, nonce, now);
        http.request(method.parse().unwrap(), format!("http://{addr}{target}"))
            .header("x-cowchat-certificate", &cert)
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .body(body)
            .send()
    };
    assert_eq!(
        send("PUT", &path, body.clone(), 1).await.unwrap().status(),
        200
    );
    assert_eq!(
        send("PUT", &path, body.clone(), 1).await.unwrap().status(),
        409
    );
    assert_eq!(
        send("PUT", &path, body.clone(), 2).await.unwrap().status(),
        200
    );
    let other = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &secret).unwrap();
    assert_ne!(wrapped, other);
    assert_eq!(
        send("PUT", &path, wire(&scope, &other, &cert, 0), 3)
            .await
            .unwrap()
            .status(),
        409
    );
    let response = send("GET", &get, vec![], 4).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let response: crate::seated::KeyEnvelope = response.json().await.unwrap();
    let received_scope = B64.decode(response.scope).unwrap();
    assert_eq!(received_scope, scope);
    let received = B64.decode(response.wrapped).unwrap();
    assert_eq!(received, wrapped);
    let public = ed25519_dalek::SigningKey::from_bytes(&seed().try_into().unwrap())
        .verifying_key()
        .to_bytes();
    key_envelope::verify(
        &received_scope,
        &received,
        &B64.decode(response.signature).unwrap(),
        &public,
    )
    .unwrap();
    let Value::Array(parts) = ciborium::from_reader(received.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(enc), Value::Bytes(ct)] = parts.as_slice() else {
        panic!()
    };
    assert_eq!(
        keys::unwrap_room_key(ROOM, 0, &hpke("recipient_private_hex"), enc, ct).unwrap(),
        secret
    );
    assert_eq!(send("GET", &get, vec![1], 5).await.unwrap().status(), 401);
    // No plaintext was stored by the envelope service; only exact sealed bytes.
    let stored: Vec<u8> = store
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT wrapped FROM seated_key_envelopes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, wrapped);
    assert!(!stored.windows(secret.len()).any(|w| w == secret));
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM seated_credentials", [])
        .unwrap();
    assert_eq!(send("GET", &get, vec![], 6).await.unwrap().status(), 401);
    assert_eq!(send("PUT", &path, body, 7).await.unwrap().status(), 401);
    server.abort();
}

#[test]
fn key_envelope_rejects_resigned_wrong_scope_and_tampering_without_consuming_nonce() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    let (seat, cert) = setup(&store, now);
    let scope = make_scope(&seat, &cert);
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &[42; 32]).unwrap();
    let path = format!("/rooms/{ROOM}/key-envelopes/{cert}/0");
    for (name, value) in [
        ("chain_id", 2u64.into()),
        ("room", text(ID)),
        ("recipient_seat", text(SEAT)),
        ("recipient_cert", text("other")),
        ("recipient_key", Value::Bytes(vec![9; 32])),
        ("auth_generation", 1u64.into()),
        ("key_generation", 1u64.into()),
        ("transport_generation", 1u64.into()),
        ("publisher_cert", text("other")),
    ] {
        let wrong = replace(&scope, name, value);
        let input = wire(&wrong, &wrapped, &cert, 0);
        assert!(
            matches!(
                call(&store, &cert, "PUT", &path, &input, 10, now),
                Err(StoreError::SeatedAuthorization)
            ),
            "{name}"
        );
    }
    let input = wire(&scope, &wrapped, &cert, 0);
    let mut tampered: serde_json::Value = serde_json::from_slice(&input).unwrap();
    tampered["signature"] = B64.encode([0; 64]).into();
    assert!(call(
        &store,
        &cert,
        "PUT",
        &path,
        &serde_json::to_vec(&tampered).unwrap(),
        10,
        now
    )
    .is_err());
    call(&store, &cert, "PUT", &path, &input, 10, now).unwrap();
    assert!(call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=1"),
        b"",
        11,
        now
    )
    .is_err());
    // An enrolled identity cannot be substituted even with a correct caller signature.
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_credentials SET identity_certificate=X''", [])
        .unwrap();
    assert!(call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=0"),
        b"",
        11,
        now
    )
    .is_err());
}

#[test]
fn key_envelope_survives_restart_and_transport_change_requires_new_current_scope() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("room.sqlite");
    let now = Utc::now().timestamp_millis();
    let store = Store::open(&db).unwrap();
    let (seat, cert) = setup(&store, now);
    let scope = make_scope(&seat, &cert);
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &[42; 32]).unwrap();
    let path = format!("/rooms/{ROOM}/key-envelopes/{cert}/0");
    let input = wire(&scope, &wrapped, &cert, 0);
    call(&store, &cert, "PUT", &path, &input, 20, now).unwrap();
    drop(store);
    let store = Store::open(&db).unwrap();
    call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=0"),
        b"",
        21,
        now,
    )
    .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET transport_generation=1", [])
        .unwrap();
    assert!(call(&store, &cert, "PUT", &path, &input, 22, now).is_err());
    assert!(call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=0"),
        b"",
        22,
        now
    )
    .is_err());
    let updated = wire(
        &replace(&scope, "transport_generation", 1u64.into()),
        &wrapped,
        &cert,
        1,
    );
    call(&store, &cert, "PUT", &path, &updated, 22, now).unwrap();
    call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=1"),
        b"",
        23,
        now,
    )
    .unwrap();
    assert!(call(
        &store,
        &cert,
        "GET",
        &format!("{path}?transport_generation=1"),
        b"",
        24,
        now + 60_001
    )
    .is_err());
}

#[test]
fn key_envelope_builder_recipient_cannot_publish_or_cross_read_and_membership_floor_is_live() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    let (seat, owner) = setup(&store, now);
    let builder_seed = [17; 32];
    let builder_public = ed25519_dalek::SigningKey::from_bytes(&builder_seed)
        .verifying_key()
        .to_bytes();
    // Explicit already-authenticated fixture for the missing builder enrollment
    // factory. This tests delivery authorization, not production builder issuance.
    let (raw_identity, raw_context): (Vec<u8>, Vec<u8>) = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT identity_certificate,trusted_context FROM seated_credentials WHERE cert_id=?1",
            [&owner],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let identity = replace(
        &replace(&raw_identity, "role", text("builder")),
        "pubkey",
        Value::Bytes(builder_public.to_vec()),
    );
    let id = certificates::certificate_id(certificates::IDENTITY, &identity).unwrap();
    let builder = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let builder_seat = format!("{seat}#builder");
    let mut context = raw_context;
    for (key, value) in [
        ("role", text("builder")),
        ("seat", text(&builder_seat)),
        ("cert", text(&builder)),
        ("public_key", Value::Bytes(builder_public.to_vec())),
        ("rights", Value::Array(vec![text("read"), text("write")])),
    ] {
        context = replace(&context, key, value);
    }
    store.conn.lock().unwrap().execute("INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context,identity_certificate) VALUES (?1,?2,?3,'Builder',0,?4,?5,?6)",params![ROOM,builder,builder_seat,builder_public.to_vec(),context,identity]).unwrap();
    let scope = replace(
        &replace(&make_scope(&seat, &owner), "recipient_cert", text(&builder)),
        "recipient_seat",
        text(&builder_seat),
    );
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &[42; 32]).unwrap();
    let input = wire(&scope, &wrapped, &owner, 0);
    let path = format!("/rooms/{ROOM}/key-envelopes/{builder}/0");
    let target = format!("{path}?transport_generation=0");
    let invoke =
        |caller: &str, method: &str, target: &str, body: &[u8], nonce: u8, signing_seed: &[u8]| {
            let (projection, _) = signed_for(method, target, body, nonce, now);
            let sig = request::sign(&projection, signing_seed).unwrap();
            store.seated_key_envelope(
                ROOM,
                &builder,
                "0",
                caller,
                method,
                target,
                body,
                &projection,
                &sig,
                now,
            )
        };
    invoke(&owner, "PUT", &path, &input, 30, &seed()).unwrap();
    assert!(invoke(&owner, "GET", &target, b"", 31, &seed()).is_err());
    invoke(&builder, "GET", &target, b"", 32, &builder_seed).unwrap();
    assert!(invoke(&builder, "PUT", &path, &input, 33, &builder_seed).is_err());
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE seated_credentials SET from_key_generation=1 WHERE cert_id=?1",
            [&builder],
        )
        .unwrap();
    assert!(invoke(&builder, "GET", &target, b"", 34, &builder_seed).is_err());
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM seated_credentials WHERE cert_id=?1",
            [&builder],
        )
        .unwrap();
    assert!(invoke(&owner, "PUT", &path, &input, 35, &seed()).is_err());
}

#[test]
fn key_envelope_failed_write_rolls_back_nonce_and_detached_signature_binds_both_parts() {
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now().timestamp_millis();
    let (seat, cert) = setup(&store, now);
    let scope = make_scope(&seat, &cert);
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &[42; 32]).unwrap();
    let signature = key_envelope::sign(&scope, &wrapped, &seed()).unwrap();
    let public = ed25519_dalek::SigningKey::from_bytes(&seed().try_into().unwrap())
        .verifying_key()
        .to_bytes();
    assert!(key_envelope::verify(
        &replace(&scope, "chain_id", 2u64.into()),
        &wrapped,
        &signature,
        &public
    )
    .is_err());
    let other = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &[42; 32]).unwrap();
    assert!(key_envelope::verify(&scope, &other, &signature, &public).is_err());
    assert!(key_envelope::verify(&scope, &wrapped, &signature, &[0; 32]).is_err());
    assert!(key_envelope::sign(
        &scope,
        &encode(&Value::Array(vec![
            Value::Bytes(vec![1; 32]),
            Value::Bytes(vec![1; 47])
        ])),
        &seed()
    )
    .is_err());
    let input = wire(&scope, &wrapped, &cert, 0);
    let target = format!("/rooms/{ROOM}/key-envelopes/{cert}/0");
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_key_wrap BEFORE INSERT ON seated_key_envelopes BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(call(&store, &cert, "PUT", &target, &input, 40, now).is_err());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_key_wrap;")
        .unwrap();
    call(&store, &cert, "PUT", &target, &input, 40, now).unwrap();
}
