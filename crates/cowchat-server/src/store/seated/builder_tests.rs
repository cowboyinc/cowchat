use super::key_envelope_tests::{hpke, make_scope, replace, wire};
use super::*;
use cowchat_crypto::{certificates, key_envelope, keys};
const BUILDER_SEED: [u8; 32] = [17; 32];
fn wallet_sig(raw: &[u8], kind: u32, key: u8) -> String {
    let wallet = k256::ecdsa::SigningKey::from_slice(&[key; 32]).unwrap();
    let (sig, recovery) = wallet
        .sign_prehash_recoverable(&sha3::Keccak256::digest(
            certificates::signing_bytes(kind, raw).unwrap(),
        ))
        .unwrap();
    let mut sig = sig.to_bytes().to_vec();
    sig.push(recovery.to_byte());
    B64.encode(sig)
}
fn credentials(owner: &[u8], seat: &str) -> (Vec<u8>, String) {
    let mut input: serde_json::Value = serde_json::from_slice(owner).unwrap();
    let mut identity = B64.decode(input["identity"].as_str().unwrap()).unwrap();
    let public = ed25519_dalek::SigningKey::from_bytes(&BUILDER_SEED)
        .verifying_key()
        .to_bytes();
    for (k, v) in [
        ("role", text("builder")),
        ("pubkey", Value::Bytes(public.to_vec())),
        ("enc_pubkey", Value::Bytes(hpke("recipient_public_hex"))),
    ] {
        identity = replace(&identity, k, v);
    }
    let membership = replace(
        &replace(
            &B64.decode(input["membership"].as_str().unwrap()).unwrap(),
            "seat",
            text(&format!("{seat}#builder")),
        ),
        "rights",
        Value::Array(vec![text("read"), text("write")]),
    );
    input["identity"] = B64.encode(&identity).into();
    input["identity_signature"] = wallet_sig(&identity, certificates::IDENTITY, 7).into();
    input["membership"] = B64.encode(&membership).into();
    input["membership_signature"] = wallet_sig(&membership, certificates::MEMBERSHIP, 7).into();
    let cert = certificates::certificate_id(certificates::IDENTITY, &identity)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (serde_json::to_vec(&input).unwrap(), cert)
}
fn enroll(store: &Store, body: &[u8], nonce: u8, now: i64) -> Result<(String, String), StoreError> {
    let target = format!("/rooms/{ROOM}/builder");
    let (p, _) = signed_for("POST", &target, body, nonce, now);
    let sig = request::sign(&p, &BUILDER_SEED).unwrap();
    store.enroll_seated_builder(ROOM, &target, body, &p, &sig, now)
}

#[tokio::test]
async fn builder_http_wallet_enrollment_to_owner_hpke_delivery_and_builder_post() {
    let state = crate::web::tests::test_state();
    create_empty_owned_room(&state.store);
    let now = Utc::now().timestamp_millis();
    let (owner, seat, owner_cert) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    let (body, builder_cert) = credentials(&owner, &seat);
    let builder_seat = format!("{seat}#builder");
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let http = reqwest::Client::new();
    assert_eq!(
        http.post(format!("http://{addr}/rooms/{ROOM}/owner"))
            .header("x-cowchat-key", "master")
            .body(owner)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let send =
        |method: &str, target: &str, body: Vec<u8>, cert: &str, signing_seed: &[u8], nonce| {
            let (p, _) = signed_for(method, target, &body, nonce, now);
            let sig = request::sign(&p, signing_seed).unwrap();
            http.request(method.parse().unwrap(), format!("http://{addr}{target}"))
                .header("x-cowchat-certificate", cert)
                .header("x-cowchat-request", B64.encode(p))
                .header("x-cowchat-signature", B64.encode(sig))
                .body(body)
                .send()
        };
    let target = format!("/rooms/{ROOM}/builder");
    assert_eq!(
        send("POST", &target, body.clone(), &builder_cert, &seed(), 1)
            .await
            .unwrap()
            .status(),
        401
    );
    let response = send(
        "POST",
        &target,
        body.clone(),
        &builder_cert,
        &BUILDER_SEED,
        1,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let enrollment: serde_json::Value = response.json().await.unwrap();
    assert_eq!(enrollment["seat"], builder_seat);
    assert_eq!(enrollment["cert"], builder_cert);
    assert_eq!(
        send(
            "POST",
            &target,
            body.clone(),
            &builder_cert,
            &BUILDER_SEED,
            1
        )
        .await
        .unwrap()
        .status(),
        409
    );
    assert_eq!(
        send("POST", &target, body, &builder_cert, &BUILDER_SEED, 2)
            .await
            .unwrap()
            .status(),
        200
    );
    let scope = replace(
        &replace(
            &make_scope(&seat, &owner_cert),
            "recipient_cert",
            text(&builder_cert),
        ),
        "recipient_seat",
        text(&builder_seat),
    );
    let secret = hpke("generation_secret_hex");
    let wrapped = keys::wrap_room_key(ROOM, 0, &hpke("recipient_public_hex"), &secret).unwrap();
    let path = format!("/rooms/{ROOM}/key-envelopes/{builder_cert}/0");
    assert_eq!(
        send(
            "PUT",
            &path,
            wire(&scope, &wrapped, &owner_cert, 0),
            &owner_cert,
            &seed(),
            3
        )
        .await
        .unwrap()
        .status(),
        200
    );
    let response = send(
        "GET",
        &format!("{path}?transport_generation=0"),
        vec![],
        &builder_cert,
        &BUILDER_SEED,
        4,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let response: crate::seated::KeyEnvelope = response.json().await.unwrap();
    let scope_received = B64.decode(response.scope).unwrap();
    assert_eq!(scope_received, scope);
    let received = B64.decode(response.wrapped).unwrap();
    let owner_public = ed25519_dalek::SigningKey::from_bytes(&seed().try_into().unwrap())
        .verifying_key()
        .to_bytes();
    key_envelope::verify(
        &scope_received,
        &received,
        &B64.decode(response.signature).unwrap(),
        &owner_public,
    )
    .unwrap();
    let Value::Array(parts) = ciborium::from_reader(received.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(enc), Value::Bytes(ct)] = parts.as_slice() else {
        panic!()
    };
    let recovered =
        keys::unwrap_room_key(ROOM, 0, &hpke("recipient_private_hex"), enc, ct).unwrap();
    assert_eq!(recovered, secret);
    // The enrolled builder is independently attributed and can post with the
    // delivered key through the production signed append endpoint.
    let mut header = crate::seated::SealedRecord::parse(&sealed(b"template"))
        .unwrap()
        .header;
    header.seat = builder_seat;
    header.role = "builder".into();
    header.cert = builder_cert.clone();
    header.gen = 0;
    header.mentions = vec![];
    let sealed = envelope::seal(
        &encode(&header),
        &recovered,
        b"builder used its delivered room key",
        &BUILDER_SEED,
    )
    .unwrap();
    let Value::Array(parts) = ciborium::from_reader(sealed.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(h), Value::Text(ct), Value::Bytes(sig)] = parts.as_slice() else {
        panic!()
    };
    let h: crate::seated::RecordHeader = ciborium::from_reader(h.as_slice()).unwrap();
    let mut record = serde_json::to_value(h).unwrap();
    record["body"] = ct.clone().into();
    record["sig"] = B64.encode(sig).into();
    assert_eq!(
        send(
            "POST",
            TARGET,
            serde_json::to_vec(&record).unwrap(),
            &builder_cert,
            &BUILDER_SEED,
            5
        )
        .await
        .unwrap()
        .status(),
        200
    );
    server.abort();
}

#[test]
fn builder_enrollment_rejects_wrong_wallet_scope_role_rights_and_expiry() {
    let store = Store::open_in_memory().unwrap();
    create_empty_owned_room(&store);
    let now = Utc::now().timestamp_millis();
    let (owner, seat, _) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    store
        .enroll_seated_owner(ROOM, "master", &owner, now)
        .unwrap();
    let (body, _) = credentials(&owner, &seat);
    let base: serde_json::Value = serde_json::from_slice(&body).unwrap();
    for (part, field, value) in [
        ("identity", "chain_id", 2u64.into()),
        ("identity", "gen", 1u64.into()),
        ("identity", "role", text("owner")),
        ("identity", "address", text(SEAT)),
        ("identity", "expires_at", (now as u64 - 1).into()),
        ("membership", "room", text(ID)),
        ("membership", "seat", text(&seat)),
        ("membership", "gen", 1u64.into()),
        ("membership", "from_gen", 1u64.into()),
        (
            "membership",
            "rights",
            Value::Array(vec![text("manage"), text("read"), text("write")]),
        ),
        ("membership", "expires_at", (now as u64 - 1).into()),
    ] {
        let mut input = base.clone();
        let raw = replace(
            &B64.decode(input[part].as_str().unwrap()).unwrap(),
            field,
            value,
        );
        let kind = if part == "identity" {
            certificates::IDENTITY
        } else {
            certificates::MEMBERSHIP
        };
        input[part] = B64.encode(&raw).into();
        input[format!("{part}_signature")] = wallet_sig(&raw, kind, 7).into();
        assert!(
            enroll(&store, &serde_json::to_vec(&input).unwrap(), 10, now).is_err(),
            "{part}.{field}"
        );
    }
    for part in ["identity", "membership"] {
        let mut input = base.clone();
        let kind = if part == "identity" {
            certificates::IDENTITY
        } else {
            certificates::MEMBERSHIP
        };
        input[format!("{part}_signature")] =
            wallet_sig(&B64.decode(input[part].as_str().unwrap()).unwrap(), kind, 9).into();
        assert!(enroll(&store, &serde_json::to_vec(&input).unwrap(), 10, now).is_err());
    }
    enroll(&store, &body, 10, now).unwrap(); // All denied requests rolled back nonce.
}

#[test]
fn builder_enrollment_receipt_survives_restart_and_cannot_revive_removed_seat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("builder.sqlite");
    let store = Store::open(&path).unwrap();
    create_empty_owned_room(&store);
    let now = Utc::now().timestamp_millis();
    let (owner, seat, _) = owner_enrollment(ROOM, "owner", now as u64 + 60_000);
    store
        .enroll_seated_owner(ROOM, "master", &owner, now)
        .unwrap();
    let (body, cert) = credentials(&owner, &seat);
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_builder BEFORE INSERT ON seated_builder_enrollments BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(enroll(&store, &body, 20, now).is_err());
    let count: i64 = store
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM seated_credentials WHERE cert_id=?1",
            [&cert],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_builder;")
        .unwrap();
    enroll(&store, &body, 20, now).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    enroll(&store, &body, 21, now).unwrap();
    // A different wallet-signed identity is a renewal, outside bootstrap scope.
    let mut next: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let identity = replace(
        &B64.decode(next["identity"].as_str().unwrap()).unwrap(),
        "expires_at",
        (now as u64 + 70_000).into(),
    );
    next["identity"] = B64.encode(&identity).into();
    next["identity_signature"] = wallet_sig(&identity, certificates::IDENTITY, 7).into();
    assert!(matches!(
        enroll(&store, &serde_json::to_vec(&next).unwrap(), 22, now),
        Err(StoreError::MessageConflict)
    ));
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM seated_credentials WHERE cert_id=?1", [&cert])
        .unwrap();
    assert!(matches!(
        enroll(&store, &body, 22, now),
        Err(StoreError::SeatedAuthorization)
    ));
    assert!(matches!(
        enroll(&store, &serde_json::to_vec(&next).unwrap(), 22, now),
        Err(StoreError::MessageConflict)
    ));
}
