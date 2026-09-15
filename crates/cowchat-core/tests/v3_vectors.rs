//! Executable M0 contract, not a production room verifier. WP1/WP1b must make
//! their real caller paths pass these same independently generated fixtures.
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use ciborium::value::Value as Cbor;
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Cursor};

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
    Ok(match value {
        Cbor::Map(entries) => {
            let mut seen = HashSet::new();
            let mut sorted = Vec::new();
            for (k, v) in entries {
                // The M0 profile uses text map keys only, never floats/tags.
                if !matches!(k, Cbor::Text(_)) || !seen.insert(encode(k)) {
                    return Err("encoding");
                }
                sorted.push((k.clone(), canonical(v)?));
            }
            sorted.sort_by_key(|(k, _)| encode(k));
            Cbor::Map(sorted)
        }
        Cbor::Array(items) => Cbor::Array(items.iter().map(canonical).collect::<Result<_, _>>()?),
        Cbor::Integer(i) if u64::try_from(*i).is_ok() => value.clone(),
        Cbor::Bytes(_) | Cbor::Text(_) | Cbor::Bool(_) | Cbor::Null => value.clone(),
        _ => return Err("encoding"),
    })
}

fn decode(raw: &[u8]) -> Result<Cbor, &'static str> {
    let mut input = Cursor::new(raw);
    let value: Cbor = ciborium::from_reader(&mut input).map_err(|_| "encoding")?;
    if input.position() != raw.len() as u64 || encode(&canonical(&value)?) != raw {
        return Err("encoding");
    }
    Ok(value)
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

fn verify(v: &Value, input: &[u8]) -> Result<(), &'static str> {
    let key: [u8; 32] = bytes(v, "public_key_hex")
        .try_into()
        .map_err(|_| "signature")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "signature")?;
    let sig = Signature::from_slice(&bytes(v, "signature_hex")).map_err(|_| "signature")?;
    key.verify_strict(input, &sig).map_err(|_| "signature")
}

fn request(v: &Value) -> Result<(), &'static str> {
    let raw = bytes(v, "cbor_hex");
    let Cbor::Array(fields) = decode(&raw)? else {
        return Err("schema");
    };
    let [Cbor::Text(method), Cbor::Text(target), Cbor::Bytes(hash), Cbor::Integer(stamp), Cbor::Bytes(nonce)] =
        fields.as_slice()
    else {
        return Err("schema");
    };
    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].contains(&method.as_str())
        || !target.starts_with('/')
        || target.contains('#')
        || target.bytes().any(|b| b <= 32 || b > 126)
        || hash.len() != 32
        || nonce.len() != 16
    {
        return Err("schema");
    }
    let stamp = u64::try_from(*stamp).map_err(|_| "schema")?;
    let mut signing = b"cowchat/v3/request".to_vec();
    signing.extend_from_slice(&raw);
    verify(v, &signing)?;
    if stamp.abs_diff(v["now_ms"].as_u64().unwrap()) > 300_000 {
        return Err("timestamp");
    }
    if v["already_seen"].as_bool().unwrap() {
        return Err("replay");
    }
    assert_eq!(signing, bytes(v, "signing_hex"));
    Ok(())
}

fn envelope(v: &Value) -> Result<(), &'static str> {
    let header = v["header"].as_object().ok_or("schema")?;
    let names = [
        "v",
        "message_id",
        "chain_id",
        "room",
        "seat",
        "role",
        "via",
        "via_sender",
        "class",
        "reply_to",
        "mentions",
        "wake_hint",
        "gen",
        "cert",
        "nonce",
    ];
    if header.len() != names.len()
        || names.iter().any(|k| !header.contains_key(*k))
        || header["v"] != 3
        || header["gen"].as_u64().is_none()
        || header["chain_id"].as_u64().is_none()
        || (header["class"] == "thinking" && header["wake_hint"] != "none")
    {
        return Err("schema");
    }
    if !["message", "thinking", "system"].contains(&header["class"].as_str().ok_or("schema")?)
        || !["owner", "builder", "actor", "door", "external"]
            .contains(&header["role"].as_str().ok_or("schema")?)
        || !["none", "normal", "urgent"].contains(&header["wake_hint"].as_str().ok_or("schema")?)
    {
        return Err("schema");
    }
    // Door kinds come from the extensible gateway catalog, not a hard-coded
    // telegram/slack vocabulary. Service authorization binds a kind to a cert.
    match (&header["via"], &header["via_sender"]) {
        (Value::Null, Value::Null) => (),
        (Value::String(via), sender)
            if !via.is_empty()
                && (sender.is_null() || sender.as_str().is_some_and(|s| !s.is_empty())) => {}
        _ => return Err("schema"),
    }
    let nonce = B64
        .decode(header["nonce"].as_str().ok_or("schema")?)
        .map_err(|_| "schema")?;
    if nonce.len() != 12 {
        return Err("schema");
    }
    let aad = encode(&canonical(&from_json(&v["header"]))?);
    assert_eq!(aad, bytes(v, "header_cbor_hex"));
    let body = v["body"].as_str().ok_or("schema")?;
    let mut signing = b"cowchat/v3/envelope".to_vec();
    signing.extend_from_slice(&aad);
    signing.extend_from_slice(&Sha256::digest(body.as_bytes()));
    verify(v, &signing)?;
    let raw = B64
        .decode(body.strip_prefix("cow1:").ok_or("schema")?)
        .map_err(|_| "schema")?;
    if raw.len() < 28 || raw[..12] != nonce {
        return Err("nonce");
    }
    let key = cowchat_core::crypto::derive_room_key(
        &bytes(v, "public_test_secret_hex"),
        header["room"].as_str().ok_or("schema")?,
    );
    let cipher = ChaCha20Poly1305::new_from_slice(&key).unwrap();
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &raw[12..],
                aad: &aad,
            },
        )
        .map_err(|_| "decrypt")?;
    assert_eq!(plain, bytes(v, "plaintext_hex"));
    assert_eq!(signing, bytes(v, "signing_hex"));
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
    for mut v in fixtures("replay-sequence.json") {
        let now = v["now_ms"].as_u64().unwrap();
        seen.retain(|_, until| *until >= now);
        let Cbor::Array(fields) = decode(&bytes(&v, "cbor_hex")).unwrap() else {
            panic!()
        };
        let Cbor::Bytes(nonce) = &fields[4] else {
            panic!()
        };
        let Cbor::Integer(stamp) = fields[3] else {
            panic!()
        };
        let stamp = u64::try_from(stamp).unwrap();
        let key = (bytes(&v, "public_key_hex"), nonce.clone());
        v["already_seen"] = seen.contains_key(&key).into();
        let result = request(&v);
        if result.is_ok() {
            seen.insert(key, stamp.saturating_add(300_000));
        }
        check(&v, result);
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
    let Cbor::Map(items) = &parsed else {
        return Err("schema");
    };
    let map: std::collections::HashMap<&str, &Cbor> = items
        .iter()
        .map(|(k, v)| {
            let Cbor::Text(k) = k else { unreachable!() };
            (k.as_str(), v)
        })
        .collect();
    let kind = v["kind"].as_str().ok_or("schema")?;
    let required: &[&str] = match kind {
        "identity" => &[
            "v",
            "chain_id",
            "address",
            "pubkey",
            "enc_pubkey",
            "role",
            "aud",
            "gen",
            "expires_at",
        ],
        "membership" => &[
            "v",
            "chain_id",
            "room",
            "seat",
            "rights",
            "door_kind",
            "bound_sender",
            "from_gen",
            "gen",
            "signer_kind",
            "signer_key",
            "expires_at",
        ],
        "invocation" => &[
            "v",
            "chain_id",
            "room",
            "grantee_seat",
            "target_seat",
            "scope",
            "budget",
            "gen",
            "expires_at",
        ],
        _ => return Err("schema"),
    };
    if map.len() != required.len() || required.iter().any(|k| !map.contains_key(k)) {
        return Err("schema");
    }
    let text = |k: &str| match map.get(k) {
        Some(Cbor::Text(s)) => Ok(s.as_str()),
        _ => Err("schema"),
    };
    let uint = |k: &str| match map.get(k) {
        Some(Cbor::Integer(i)) => u64::try_from(*i).map_err(|_| "schema"),
        _ => Err("schema"),
    };
    let bin = |k: &str, len: usize| match map.get(k) {
        Some(Cbor::Bytes(b)) if b.len() == len => Ok(b.as_slice()),
        _ => Err("schema"),
    };
    if uint("v")? != 3 {
        return Err("schema");
    }
    if uint("chain_id")? != v["expected_chain_id"].as_u64().unwrap()
        || uint("gen")? != v["expected_gen"].as_u64().unwrap()
        || (kind != "identity" && text("room")? != v["expected_room"].as_str().unwrap())
    {
        return Err("scope");
    }
    match map["expires_at"] {
        Cbor::Null => (),
        Cbor::Integer(i) => {
            if u64::try_from(*i).map_err(|_| "schema")? < v["now_ms"].as_u64().unwrap() {
                return Err("expiry");
            }
        }
        _ => return Err("schema"),
    }
    let mut signer_kind = "wallet";
    match kind {
        "identity" => {
            bin("pubkey", 32)?;
            bin("enc_pubkey", 32)?;
            if text("aud")? != "cowchat" {
                return Err("scope");
            }
            let role = text("role")?;
            if !["owner", "builder", "actor", "door"].contains(&role)
                || (matches!(role, "owner" | "builder") && matches!(map["expires_at"], Cbor::Null))
                || (matches!(role, "actor" | "door") && !matches!(map["expires_at"], Cbor::Null))
            {
                return Err("schema");
            }
        }
        "membership" => {
            let Cbor::Array(rights) = map["rights"] else {
                return Err("schema");
            };
            let rights: Vec<&str> = rights
                .iter()
                .map(|r| match r {
                    Cbor::Text(s) => Ok(s.as_str()),
                    _ => Err("schema"),
                })
                .collect::<Result<_, _>>()?;
            if rights.is_empty()
                || rights
                    .iter()
                    .any(|r| !["manage", "read", "write"].contains(r))
                || rights.windows(2).any(|p| p[0] >= p[1])
            {
                return Err("schema");
            }
            match (map["door_kind"], map["bound_sender"]) {
                (Cbor::Null, Cbor::Null) => (),
                (Cbor::Text(k), Cbor::Text(s)) if !k.is_empty() && !s.is_empty() => (),
                _ => return Err("schema"),
            }
            uint("from_gen")?;
            text("seat")?;
            signer_kind = text("signer_kind")?;
            match signer_kind {
                "wallet" if matches!(map["signer_key"], Cbor::Null) => (),
                "admin" => {
                    bin("signer_key", 32)?;
                }
                _ => return Err("schema"),
            }
        }
        "invocation" => {
            // u128 wei is fixed-width bytes, not CBOR uint or a floating-point JSON number.
            let budget: [u8; 16] = bin("budget", 16)?.try_into().unwrap();
            let _wei = u128::from_be_bytes(budget);
            text("grantee_seat")?;
            text("target_seat")?;
            if text("scope")? != "wake" {
                return Err("schema");
            }
        }
        _ => unreachable!(),
    }
    let mut signed = format!("cowchat/v3/cert/{kind}").into_bytes();
    signed.extend_from_slice(&raw);
    if Sha256::digest(&signed).as_slice() != bytes(v, "cert_id_hex") {
        return Err("cert_id");
    }
    if signer_kind == "wallet" {
        use k256::ecdsa::{RecoveryId, Signature as WalletSignature, VerifyingKey as WalletKey};
        let sig = bytes(v, "signature_hex");
        if sig.len() != 65 {
            return Err("signature");
        }
        let signature = WalletSignature::from_slice(&sig[..64]).map_err(|_| "signature")?;
        if signature.normalize_s().is_some() {
            return Err("signature");
        }
        let recid = RecoveryId::from_byte(sig[64]).ok_or("signature")?;
        let digest = sha3::Keccak256::digest(&signed);
        let key =
            WalletKey::recover_from_prehash(&digest, &signature, recid).map_err(|_| "signature")?;
        let public = key.to_encoded_point(false);
        let hash = sha3::Keccak256::digest(&public.as_bytes()[1..]);
        if format!("0x{}", hex::encode(&hash[12..])) != v["expected_owner"].as_str().unwrap() {
            return Err("authority");
        }
    } else {
        verify(v, &signed)?;
        // Fixture-supplied trust context; real delegation/manage-chain and
        // finalized actor-controller verification belongs to WP1's callers.
        if bin("signer_key", 32)? != bytes(v, "public_key_hex")
            || !v["admin_has_manage"].as_bool().unwrap()
        {
            return Err("authority");
        }
    }
    assert_eq!(signed, bytes(v, "signing_hex"));
    Ok(())
}

#[test]
fn certificate_signature_and_scope_contract() {
    for v in fixtures("certificates.json") {
        check(&v, certificate(&v));
    }
}

fn unwrap_key(v: &Value) -> Result<(), &'static str> {
    use hpke::{
        aead::ChaCha20Poly1305 as HpkeChaCha, kdf::HkdfSha256, kem::X25519HkdfSha256 as Kem,
        Deserializable, Kem as _, OpModeR, Serializable,
    };
    let sk = <Kem as hpke::Kem>::PrivateKey::from_bytes(&bytes(v, "recipient_private_hex"))
        .map_err(|_| "decrypt")?;
    let enc =
        <Kem as hpke::Kem>::EncappedKey::from_bytes(&bytes(v, "enc_hex")).map_err(|_| "decrypt")?;
    let room = v["room"].as_str().unwrap();
    let mut info = b"cowchat/v3/roomkey".to_vec();
    info.extend_from_slice(&encode(&Cbor::Array(vec![
        Cbor::Text(room.into()),
        Cbor::Integer(v["gen"].as_u64().unwrap().into()),
    ])));
    let mut context =
        hpke::setup_receiver::<HpkeChaCha, HkdfSha256, Kem>(&OpModeR::Base, &sk, &enc, &info)
            .map_err(|_| "decrypt")?;
    let secret = context
        .open(&bytes(v, "ciphertext_hex"), b"")
        .map_err(|_| "decrypt")?;
    assert_eq!(secret, bytes(v, "generation_secret_hex"));
    assert_eq!(info, bytes(v, "info_hex"));
    let browser_key = cowchat_core::crypto::derive_room_key(&secret, room);
    let runner_key =
        cowchat_core::crypto::derive_room_key(&bytes(v, "cbss_released_secret_hex"), room);
    assert_eq!(browser_key, runner_key);
    assert_eq!(browser_key.as_slice(), bytes(v, "derived_aead_key_hex"));
    // The CBSS value here is a supplied public fixture, not a live release.
    assert_eq!(
        Kem::sk_to_pk(&sk).to_bytes().as_slice(),
        bytes(v, "recipient_public_hex")
    );
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
