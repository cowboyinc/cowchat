use ciborium::value::Value;
use cowchat_crypto::{canonical, envelope, native_actor, Error};
use ed25519_dalek::SigningKey;

fn encode(value: Value) -> Vec<u8> {
    let mut raw = Vec::new();
    ciborium::into_writer(&value, &mut raw).unwrap();
    canonical::canonicalize(&raw).unwrap()
}

fn source_input_header(
    chain_id: u64,
    room: [u8; 32],
    source: [u8; 32],
    target: [u8; 32],
    cert: [u8; 32],
    generation: u64,
    role: &str,
) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::Text("v".into()), Value::Integer(3.into())),
        (
            Value::Text("message_id".into()),
            Value::Text(hex::encode([0x66; 32])),
        ),
        (
            Value::Text("chain_id".into()),
            Value::Integer(chain_id.into()),
        ),
        (Value::Text("room".into()), Value::Text(hex::encode(room))),
        (Value::Text("seat".into()), Value::Text(hex::encode(source))),
        (Value::Text("role".into()), Value::Text(role.into())),
        (Value::Text("via".into()), Value::Null),
        (Value::Text("via_sender".into()), Value::Null),
        (Value::Text("class".into()), Value::Text("message".into())),
        (Value::Text("reply_to".into()), Value::Null),
        (
            Value::Text("mentions".into()),
            Value::Array(vec![Value::Text(hex::encode(target))]),
        ),
        (
            Value::Text("wake_hint".into()),
            Value::Text("normal".into()),
        ),
        (Value::Text("gen".into()), Value::Integer(generation.into())),
        (Value::Text("cert".into()), Value::Text(hex::encode(cert))),
        (
            Value::Text("nonce".into()),
            Value::Text("AAAAAAAAAAAAAAAA".into()),
        ),
    ]))
}

#[test]
fn actor_input_is_bound_to_finalized_identity_before_plaintext() {
    let room = [0x11; 32];
    let source = [0x22; 32];
    let target = [0x33; 32];
    let cert = [0x44; 32];
    let generation_secret = [0x55; 32];
    let seed = [0x77; 32];
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let header = source_input_header(31337, room, source, target, cert, 9, "actor");
    let sealed = envelope::seal(&header, &generation_secret, b"private trigger", &seed).unwrap();
    let expected = native_actor::ExpectedSourceSeatRecordV1 {
        chain_id: 31337,
        room_id: room,
        source_seat_id: source,
        source_seat_kind: native_actor::SourceSeatKindV1::Actor,
        source_key_binding_commitment: cert,
        key_generation: 9,
        target_seat_id: target,
    };

    let opened =
        native_actor::open_source_seat_record_v1(&sealed, &expected, &public, &generation_secret)
            .unwrap();
    assert_eq!(opened.message_id, [0x66; 32]);
    assert_eq!(opened.reply_to, None);
    assert_eq!(opened.mentions, vec![target]);
    assert_eq!(opened.plaintext, b"private trigger");

    let mut wrong_room = expected.clone();
    wrong_room.room_id[0] ^= 1;
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &wrong_room, &public, &[0; 32]),
        Err(Error::Scope),
        "authority mismatch must win before decryption"
    );
    let mut wrong_cert = expected.clone();
    wrong_cert.source_key_binding_commitment[0] ^= 1;
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &wrong_cert, &public, &generation_secret),
        Err(Error::Scope)
    );
    let mut wrong_target = expected.clone();
    wrong_target.target_seat_id[0] ^= 1;
    assert_eq!(
        native_actor::open_source_seat_record_v1(
            &sealed,
            &wrong_target,
            &public,
            &generation_secret
        ),
        Err(Error::Scope)
    );
    assert_eq!(
        native_actor::open_source_seat_record_v1(
            &sealed,
            &expected,
            &[0x99; 32],
            &generation_secret
        ),
        Err(Error::Signature)
    );
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &expected, &public, &[0; 32]),
        Err(Error::Decrypt)
    );
}

#[test]
fn human_input_uses_the_finalized_seat_key_and_gateway_input_is_refused() {
    let room = [0x11; 32];
    let source = [0x22; 32];
    let target = [0x33; 32];
    let cert = [0x44; 32];
    let generation_secret = [0x55; 32];
    let seed = [0x77; 32];
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let header = source_input_header(31337, room, source, target, cert, 9, "human");
    let sealed = envelope::seal(&header, &generation_secret, b"human trigger", &seed).unwrap();
    let mut expected = native_actor::ExpectedSourceSeatRecordV1 {
        chain_id: 31337,
        room_id: room,
        source_seat_id: source,
        source_seat_kind: native_actor::SourceSeatKindV1::Human,
        source_key_binding_commitment: cert,
        key_generation: 9,
        target_seat_id: target,
    };

    let opened =
        native_actor::open_source_seat_record_v1(&sealed, &expected, &public, &generation_secret)
            .unwrap();
    assert_eq!(opened.plaintext, b"human trigger");

    expected.source_seat_kind = native_actor::SourceSeatKindV1::Actor;
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &expected, &public, &generation_secret),
        Err(Error::Scope),
        "the signed header role must match finalized principal_kind"
    );

    expected.source_seat_kind = native_actor::SourceSeatKindV1::Gateway;
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &expected, &[0; 32], &[0; 32]),
        Err(Error::Scope),
        "gateway attribution requires its own ingress profile"
    );
}

#[test]
fn actor_reply_builder_is_exact_and_signer_bound() {
    let seed = [0x88; 32];
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let generation_secret = [0x99; 32];
    let expected = native_actor::ActorReplyHeaderV1 {
        chain_id: 31337,
        room_id: [0x11; 32],
        seat_id: [0x22; 32],
        key_binding_commitment: [0x33; 32],
        key_generation: 12,
        message_id: [0x44; 32],
        reply_to: [0x55; 32],
    };
    assert_eq!(
        hex::encode(native_actor::actor_reply_header_v1(&expected).unwrap()),
        "af6176036367656e0c63766961f6646365727478403333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333364726f6c65656163746f7264726f6f6d784031313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131647365617478403232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323265636c617373676d657373616765656e6f6e6365704141414141414141414141414141414168636861696e5f6964197a69686d656e74696f6e7380687265706c795f746f7840353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535353535356977616b655f68696e74646e6f6e656a6d6573736167655f69647840343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434343434346a7669615f73656e646572f6"
    );
    let sealed = native_actor::seal_actor_reply_v1(
        &expected,
        &generation_secret,
        b"private reply",
        &seed,
        &public,
    )
    .unwrap();
    assert_eq!(
        native_actor::authenticated_record_message_id_v1(&sealed, &public).unwrap(),
        expected.message_id
    );
    assert_eq!(
        native_actor::authenticated_record_message_id_v1(&sealed, &[0x77; 32]),
        Err(Error::Signature)
    );
    assert_eq!(
        native_actor::open_actor_reply_v1(&sealed, &expected, &public, &generation_secret).unwrap(),
        b"private reply"
    );

    let mut wrong_message = expected.clone();
    wrong_message.message_id[0] ^= 1;
    assert_eq!(
        native_actor::open_actor_reply_v1(&sealed, &wrong_message, &public, &[0; 32]),
        Err(Error::Scope),
        "stable identity mismatch must win before decryption"
    );
    let wrong_seed = [0xaa; 32];
    assert_eq!(
        native_actor::seal_actor_reply_v1(
            &expected,
            &generation_secret,
            b"private reply",
            &wrong_seed,
            &public,
        ),
        Err(Error::Authority)
    );
}

#[test]
fn native_actor_profile_rejects_noncanonical_identity_text() {
    let room = [0x11; 32];
    let source = [0x22; 32];
    let target = [0x33; 32];
    let cert = [0xab; 32];
    let secret = [0x55; 32];
    let seed = [0x77; 32];
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let mut value: Value = ciborium::from_reader(
        source_input_header(31337, room, source, target, cert, 9, "actor").as_slice(),
    )
    .unwrap();
    let Value::Map(fields) = &mut value else {
        panic!()
    };
    let cert_value = fields
        .iter_mut()
        .find(|(key, _)| key == &Value::Text("cert".into()))
        .unwrap();
    cert_value.1 = Value::Text(hex::encode(cert).to_uppercase());
    let header = encode(value);
    let sealed = envelope::seal(&header, &secret, b"private trigger", &seed).unwrap();
    let expected = native_actor::ExpectedSourceSeatRecordV1 {
        chain_id: 31337,
        room_id: room,
        source_seat_id: source,
        source_seat_kind: native_actor::SourceSeatKindV1::Actor,
        source_key_binding_commitment: cert,
        key_generation: 9,
        target_seat_id: target,
    };
    assert_eq!(
        native_actor::open_source_seat_record_v1(&sealed, &expected, &public, &secret),
        Err(Error::Scope)
    );
}
