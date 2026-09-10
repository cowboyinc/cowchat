use ciborium::value::Value as Cbor;
use cowchat_crypto::{canonical, certificates, envelope, keys, request, Error};
use serde_json::Value;

fn fixture(file: &str, name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/v3")
        .join(file);
    let values: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    values.into_iter().find(|v| v["id"] == name).unwrap()
}
fn bytes(v: &Value, name: &str) -> Vec<u8> {
    hex::decode(v[name].as_str().unwrap()).unwrap()
}
fn encode(v: &Cbor) -> Vec<u8> {
    let mut raw = Vec::new();
    ciborium::into_writer(v, &mut raw).unwrap();
    canonical::canonicalize(&raw).unwrap()
}
fn decode(raw: &[u8]) -> Cbor {
    ciborium::from_reader(raw).unwrap()
}

#[test]
fn hostile_declared_lengths_and_nesting_fail_before_decode() {
    assert_eq!(
        canonical::validate(&[0x9b, 255, 255, 255, 255, 255, 255, 255, 255]),
        Err(Error::Limit)
    );
    assert!(canonical::validate(&[0x5b, 255, 255, 255, 255, 255, 255, 255, 255]).is_err());
    assert_eq!(
        canonical::validate(&vec![0; canonical::MAX_BYTES + 1]),
        Err(Error::Limit)
    );
    let mut deep = vec![0x81; canonical::MAX_DEPTH + 1];
    deep.push(0xf6);
    assert_eq!(canonical::validate(&deep), Err(Error::Limit));
    let mut boundary = vec![0x81; canonical::MAX_DEPTH];
    boundary.push(0xf6);
    assert!(canonical::validate(&boundary).is_ok());
    // Array count alone exceeds the value budget, before reading its elements.
    assert_eq!(canonical::validate(&[0x99, 0x10, 0x00]), Err(Error::Limit));
}

#[test]
fn malformed_profile_inputs_and_duplicate_keys_fail() {
    for input in [
        vec![0xa1, 0x01, 0x01],
        vec![0xf9, 0, 0],
        vec![0xc0, 0x01],
        vec![0x20],
        vec![0xa2, 0x61, b'a', 1, 0x61, b'a', 2],
        vec![0x61, 0xff],
    ] {
        assert!(canonical::canonicalize(&input).is_err(), "{input:x?}");
    }
}

#[test]
fn received_http_bytes_are_bound_to_signature() {
    let v = fixture("requests.json", "raw-query-duplicates");
    let raw = bytes(&v, "cbor_hex");
    let public = bytes(&v, "public_key_hex");
    let sig = bytes(&v, "signature_hex");
    let now = v["now_ms"].as_u64().unwrap();
    let target = "/rooms/test/messages?a=%2F&a=/";
    assert!(request::verify_received(&raw, &public, &sig, now, "GET", target, b"").is_ok());
    assert_eq!(
        request::verify_received(&raw, &public, &sig, now, "POST", target, b""),
        Err(Error::Scope)
    );
    assert_eq!(
        request::verify_received(
            &raw,
            &public,
            &sig,
            now,
            "GET",
            "/rooms/test/messages?a=/&a=/",
            b""
        ),
        Err(Error::Scope)
    );
    assert_eq!(
        request::verify_received(&raw, &public, &sig, now, "GET", target, b"changed"),
        Err(Error::Scope)
    );
    assert_eq!(
        request::sign(&raw, &(0..32).collect::<Vec<u8>>()).unwrap(),
        sig
    );
    assert_eq!(
        request::verify_projection(&raw, &public[..31], &sig, now),
        Err(Error::Signature)
    );
}

#[test]
fn fresh_seals_open_and_retry_bytes_are_stable() {
    let v = fixture("envelopes.json", "owner-message");
    let header = bytes(&v, "header_cbor_hex");
    let secret = bytes(&v, "public_test_secret_hex");
    let seed = (0..32).collect::<Vec<u8>>();
    let public = bytes(&v, "public_key_hex");
    let a = envelope::seal(&header, &secret, b"new private message", &seed).unwrap();
    let b = envelope::seal(&header, &secret, b"new private message", &seed).unwrap();
    assert_ne!(a, b);
    let Cbor::Array(fields) = decode(&a) else {
        panic!()
    };
    let [Cbor::Bytes(header), Cbor::Text(body), Cbor::Bytes(sig)] = fields.as_slice() else {
        panic!()
    };
    for _ in 0..2 {
        assert_eq!(
            envelope::open(header, body, &public, sig, &secret).unwrap(),
            b"new private message"
        );
    }
    assert_eq!(
        envelope::open(header, body, &public, sig, &[0; 32]),
        Err(Error::Decrypt)
    );
    assert_eq!(
        envelope::open(header, body, &public, &[0; 64], &secret),
        Err(Error::Signature)
    );
    assert_eq!(
        envelope::signing_bytes(header, &"x".repeat(envelope::MAX_BODY_BYTES + 1)),
        Err(Error::Limit)
    );
}

#[test]
fn hpke_wraps_use_fresh_ephemeral_keys() {
    let v = fixture("hpke.json", "generation-secret");
    let private = bytes(&v, "recipient_private_hex");
    let public = bytes(&v, "recipient_public_hex");
    let secret = bytes(&v, "generation_secret_hex");
    let a = keys::wrap_room_key("test-room", 2, &public, &secret).unwrap();
    let b = keys::wrap_room_key("test-room", 2, &public, &secret).unwrap();
    assert_ne!(a, b);
    let Cbor::Array(fields) = decode(&a) else {
        panic!()
    };
    let [Cbor::Bytes(enc), Cbor::Bytes(ct)] = fields.as_slice() else {
        panic!()
    };
    assert_eq!(
        keys::unwrap_room_key("test-room", 2, &private, enc, ct).unwrap(),
        secret
    );
    assert_eq!(
        keys::unwrap_room_key("test-room", 3, &private, enc, ct),
        Err(Error::Decrypt)
    );
    assert_eq!(
        keys::wrap_room_key("test-room", 2, &public, &secret[..31]),
        Err(Error::Schema)
    );
}

#[test]
fn admin_signature_does_not_supply_its_own_authority() {
    let v = fixture("certificates.json", "membership-admin");
    let cert = bytes(&v, "cbor_hex");
    let sig = bytes(&v, "signature_hex");
    let id = bytes(&v, "cert_id_hex");
    let owner = hex::decode(
        v["expected_owner"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
    )
    .unwrap();
    let mut fields = vec![
        (Cbor::Text("chain_id".into()), Cbor::Integer(1.into())),
        (Cbor::Text("room".into()), Cbor::Text("test-room".into())),
        (Cbor::Text("gen".into()), Cbor::Integer(2.into())),
        (
            Cbor::Text("now_ms".into()),
            Cbor::Integer(v["now_ms"].as_u64().unwrap().into()),
        ),
        (Cbor::Text("wallet_address".into()), Cbor::Bytes(owner)),
        (Cbor::Text("admin_key".into()), Cbor::Null),
    ];
    assert_eq!(
        certificates::verify(2, &cert, &sig, &id, &encode(&Cbor::Map(fields.clone()))),
        Err(Error::Authority)
    );
    fields[5].1 = Cbor::Bytes(bytes(&v, "public_key_hex"));
    assert!(certificates::verify(2, &cert, &sig, &id, &encode(&Cbor::Map(fields.clone()))).is_ok());
    fields[3].1 = Cbor::Null;
    assert_eq!(
        certificates::verify(2, &cert, &sig, &id, &encode(&Cbor::Map(fields))),
        Err(Error::Schema)
    );
    assert_eq!(
        certificates::sign_admin(&cert, &(0..32).collect::<Vec<u8>>()).unwrap(),
        sig
    );
}
