//! Fixed M0 vectors now exercise production cryptography and codec APIs.
//! Only supplied trust and replay state remain in the harness, not live state.
use ciborium::value::Value as Cbor;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

fn fixtures(name: &str) -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/v3")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn bytes(v: &Value, field: &str) -> Vec<u8> {
    hex::decode(v[field].as_str().unwrap()).unwrap()
}

fn encode(value: &Cbor) -> Vec<u8> {
    let mut result = Vec::new();
    ciborium::into_writer(value, &mut result).unwrap();
    result
}

fn canonical(value: &Cbor) -> Result<Cbor, &'static str> {
    let out = cowchat_crypto::canonical::canonicalize(&encode(value)).map_err(|e| e.name())?;
    ciborium::from_reader(out.as_slice()).map_err(|_| "encoding")
}
fn decode(raw: &[u8]) -> Result<Cbor, &'static str> {
    cowchat_crypto::canonical::validate(raw).map_err(|e| e.name())?;
    ciborium::from_reader(raw).map_err(|_| "encoding")
}

fn from_json(value: &Value) -> Cbor {
    match value {
        Value::Null => Cbor::Null,
        Value::Bool(v) => Cbor::Bool(*v),
        Value::String(v) => Cbor::Text(v.clone()),
        Value::Number(v) => Cbor::Integer(v.as_u64().unwrap().into()),
        Value::Array(v) => Cbor::Array(v.iter().map(from_json).collect()),
        Value::Object(v) => Cbor::Map(
            v.iter()
                .map(|(k, v)| (Cbor::Text(k.clone()), from_json(v)))
                .collect(),
        ),
    }
}

fn request(v: &Value) -> Result<(), &'static str> {
    let raw = bytes(v, "cbor_hex");
    cowchat_crypto::request::verify_projection(
        &raw,
        &bytes(v, "public_key_hex"),
        &bytes(v, "signature_hex"),
        v["now_ms"].as_u64().unwrap(),
    )
    .map_err(|e| e.name())?;
    if v["already_seen"].as_bool().unwrap() {
        return Err("replay");
    }
    assert_eq!(
        cowchat_crypto::request::signing_bytes(&raw).unwrap(),
        bytes(v, "signing_hex")
    );
    Ok(())
}
fn envelope(v: &Value) -> Result<(), &'static str> {
    let aad = encode(&canonical(&from_json(&v["header"]))?);
    assert_eq!(aad, bytes(v, "header_cbor_hex"));
    let body = v["body"].as_str().unwrap();
    let plaintext = cowchat_crypto::envelope::open(
        &aad,
        body,
        &bytes(v, "public_key_hex"),
        &bytes(v, "signature_hex"),
        &bytes(v, "public_test_secret_hex"),
    )
    .map_err(|e| e.name())?;
    assert_eq!(plaintext, bytes(v, "plaintext_hex"));
    assert_eq!(
        cowchat_crypto::envelope::signing_bytes(&aad, body).unwrap(),
        bytes(v, "signing_hex")
    );
    Ok(())
}

fn check(v: &Value, result: Result<(), &'static str>) {
    assert_eq!(
        result.err().unwrap_or("accept"),
        v["expected"].as_str().unwrap(),
        "{}",
        v["id"]
    );
}

#[test]
fn deterministic_encoding_vectors() {
    for v in fixtures("encoding.json") {
        let result = decode(&bytes(&v, "cbor_hex")).map(|parsed| {
            assert_eq!(parsed, canonical(&from_json(&v["input"])).unwrap());
        });
        check(&v, result);
    }
}

#[test]
fn request_signature_and_replay_contract() {
    for v in fixtures("requests.json") {
        check(&v, request(&v));
    }
}

#[test]
fn envelope_signature_and_aad_contract() {
    for v in fixtures("envelopes.json") {
        check(&v, envelope(&v));
    }
}

#[test]
fn future_timestamp_nonce_retention() {
    let mut seen = std::collections::HashMap::new();
    for v in fixtures("replay-sequence.json") {
        let now = v["now_ms"].as_u64().unwrap();
        seen.retain(|_, until| *until >= now);
        let result = cowchat_crypto::request::verify_projection(
            &bytes(&v, "cbor_hex"),
            &bytes(&v, "public_key_hex"),
            &bytes(&v, "signature_hex"),
            now,
        );
        match result {
            Err(e) => check(&v, Err(e.name())),
            Ok(token) => {
                let Cbor::Array(fields) = decode(&token).unwrap() else {
                    panic!()
                };
                let [Cbor::Bytes(key), Cbor::Bytes(nonce), Cbor::Integer(until)] =
                    fields.as_slice()
                else {
                    panic!()
                };
                let pair = (key.clone(), nonce.clone());
                match seen.entry(pair) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        assert_eq!(v["expected"], "replay");
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(u64::try_from(*until).unwrap());
                        assert_eq!(v["expected"], "accept");
                    }
                }
            }
        }
    }
}

fn certificate(v: &Value) -> Result<(), &'static str> {
    let raw = bytes(v, "cbor_hex");
    let parsed = decode(&raw)?;
    let mut projected = from_json(&v["fields"]);
    if let Cbor::Map(fields) = &mut projected {
        for (key, value) in fields {
            let Cbor::Text(key) = key else { unreachable!() };
            if v["byte_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k == key)
            {
                let Cbor::Text(hex) = value else {
                    panic!("typed hex fixture")
                };
                *value = Cbor::Bytes(hex::decode(hex).unwrap());
            } else if key == "budget" {
                if let Cbor::Text(decimal) = value {
                    *value = Cbor::Bytes(decimal.parse::<u128>().unwrap().to_be_bytes().to_vec());
                }
            }
        }
    }
    assert_eq!(
        parsed,
        canonical(&projected).unwrap(),
        "fixture projection {}",
        v["id"]
    );
    let kind = match v["kind"].as_str().unwrap() {
        "identity" => 1,
        "membership" => 2,
        "invocation" => 3,
        _ => panic!(),
    };
    // Caller-supplied trust fixture. Production has no admin_has_manage flag:
    // its admin key must be resolved through an authenticated manage chain.
    let admin = if v["admin_has_manage"] == true {
        Cbor::Bytes(bytes(v, "public_key_hex"))
    } else {
        Cbor::Null
    };
    let context = encode(&canonical(&Cbor::Map(vec![
        (
            Cbor::Text("chain_id".into()),
            from_json(&v["expected_chain_id"]),
        ),
        (Cbor::Text("room".into()), from_json(&v["expected_room"])),
        (Cbor::Text("gen".into()), from_json(&v["expected_gen"])),
        (Cbor::Text("now_ms".into()), from_json(&v["now_ms"])),
        (
            Cbor::Text("wallet_address".into()),
            Cbor::Bytes(
                hex::decode(
                    v["expected_owner"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("0x"),
                )
                .unwrap(),
            ),
        ),
        (Cbor::Text("admin_key".into()), admin),
    ]))?);
    cowchat_crypto::certificates::verify(
        kind,
        &raw,
        &bytes(v, "signature_hex"),
        &bytes(v, "cert_id_hex"),
        &context,
    )
    .map_err(|e| e.name())?;
    assert_eq!(
        cowchat_crypto::certificates::signing_bytes(kind, &raw).unwrap(),
        bytes(v, "signing_hex")
    );
    Ok(())
}

#[test]
fn certificate_signature_and_scope_contract() {
    for v in fixtures("certificates.json") {
        check(&v, certificate(&v));
    }
}

fn unwrap_key(v: &Value) -> Result<(), &'static str> {
    let room = v["room"].as_str().unwrap();
    let secret = cowchat_crypto::keys::unwrap_room_key(
        room,
        v["gen"].as_u64().unwrap(),
        &bytes(v, "recipient_private_hex"),
        &bytes(v, "enc_hex"),
        &bytes(v, "ciphertext_hex"),
    )
    .map_err(|e| e.name())?;
    assert_eq!(secret, bytes(v, "generation_secret_hex"));
    let browser = cowchat_crypto::keys::derive_room_key(&secret, room);
    let runner = cowchat_crypto::keys::derive_room_key(&bytes(v, "cbss_released_secret_hex"), room);
    assert_eq!(browser, runner);
    assert_eq!(browser.as_slice(), bytes(v, "derived_aead_key_hex"));
    Ok(())
}

#[test]
fn hpke_browser_and_cbss_input_parity() {
    for v in fixtures("hpke.json") {
        check(&v, unwrap_key(&v));
    }
}

#[test]
fn manifest_binds_every_vector_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/v3");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut ids = HashSet::new();
    for item in manifest["files"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        assert!(ids.insert(path));
        let raw = std::fs::read(root.join(path)).unwrap();
        assert_eq!(hex::encode(Sha256::digest(&raw)), item["sha256"]);
        let cases: Vec<Value> = serde_json::from_slice(&raw).unwrap();
        assert_eq!(cases.len() as u64, item["cases"].as_u64().unwrap());
        let mut names = HashSet::new();
        for case in cases {
            assert!(names.insert(case["id"].as_str().unwrap().to_owned()));
        }
    }
    assert_eq!(ids.len(), 6);
}
