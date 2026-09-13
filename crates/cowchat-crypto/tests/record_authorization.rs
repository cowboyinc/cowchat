use ciborium::value::Value;
use cowchat_crypto::{authorization, canonical, envelope, Error};

fn encode(value: Value) -> Result<Vec<u8>, Error> {
    let mut raw = Vec::new();
    ciborium::into_writer(&value, &mut raw).unwrap();
    canonical::canonicalize(&raw)
}
fn decode(raw: &[u8]) -> Result<Value, Error> {
    canonical::validate(raw)?;
    ciborium::from_reader(raw).map_err(|_| Error::Encoding)
}

fn put(map: &mut Value, name: &str, value: Value) {
    let Value::Map(fields) = map else { panic!() };
    if let Some((_, old)) = fields
        .iter_mut()
        .find(|(key, _)| key == &Value::Text(name.into()))
    {
        *old = value;
    } else {
        fields.push((Value::Text(name.into()), value));
    }
}
fn text(value: &str) -> Value {
    Value::Text(value.into())
}
fn owner_fixture() -> (Value, Value) {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/v3/envelopes.json")).unwrap();
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture["id"] == "owner-message")
        .unwrap();
    let header =
        decode(&hex::decode(fixture["header_cbor_hex"].as_str().unwrap()).unwrap()).unwrap();
    let public = hex::decode(fixture["public_key_hex"].as_str().unwrap()).unwrap();
    let context = Value::Map(vec![
        (text("chain_id"), 1u64.into()),
        (text("room"), text("test-room")),
        (text("gen"), 2u64.into()),
        (
            text("seat"),
            text("0x1111111111111111111111111111111111111111"),
        ),
        (text("role"), text("owner")),
        (text("cert"), text("test-cert")),
        (text("public_key"), Value::Bytes(public)),
        (
            text("rights"),
            Value::Array(vec![text("read"), text("write")]),
        ),
        (text("expires_at"), 10_000u64.into()),
        (text("door_kind"), Value::Null),
        (text("bound_sender"), Value::Null),
        (text("forwarded_seat"), Value::Null),
    ]);
    (header, context)
}
fn verify(header: Value, context: Value) -> Result<(), Error> {
    let seed: Vec<_> = (0..32).collect();
    let raw = envelope::seal(&encode(header).unwrap(), &[42; 32], b"private body", &seed)?;
    let Value::Array(parts) = ciborium::from_reader(raw.as_slice()).unwrap() else {
        panic!()
    };
    let [Value::Bytes(header), Value::Text(body), Value::Bytes(signature)] = parts.as_slice()
    else {
        panic!()
    };
    authorization::verify_member_record(header, body, signature, &encode(context).unwrap(), 1000)
}

#[test]
fn authenticated_context_binds_network_room_generation_certificate_rights_and_expiry() {
    let (header, context) = owner_fixture();
    assert_eq!(verify(header.clone(), context.clone()), Ok(()));
    for (field, value, expected) in [
        ("chain_id", 2u64.into(), Error::Scope),
        ("room", text("another-room"), Error::Scope),
        ("gen", 3u64.into(), Error::Scope),
        ("cert", text("another-cert"), Error::Scope),
        ("rights", Value::Array(vec![text("read")]), Error::Authority),
        (
            "rights",
            Value::Array(vec![text("write"), text("write")]),
            Error::Authority,
        ),
        ("expires_at", 999u64.into(), Error::Expiry),
        ("seat", text("other-seat"), Error::Authority),
        ("role", text("actor"), Error::Authority),
    ] {
        let mut changed = context.clone();
        put(&mut changed, field, value);
        assert_eq!(verify(header.clone(), changed), Err(expected), "{field}");
    }
}

#[test]
fn correctly_signed_member_cannot_impersonate_another_seat_or_emit_system_records() {
    let (header, context) = owner_fixture();
    for (field, value) in [
        (
            "seat",
            text("0x1111111111111111111111111111111111111111#builder"),
        ),
        ("role", text("builder")),
        ("class", text("system")),
        ("via_sender", text("forged-external-sender")),
    ] {
        let mut changed = header.clone();
        put(&mut changed, field, value);
        // Each mutation is freshly encrypted and correctly signed by the valid
        // member key. Signature verification alone would accept these records.
        assert_eq!(
            verify(changed, context.clone()),
            Err(Error::Authority),
            "{field}"
        );
    }
}

#[test]
fn door_forwarding_is_bound_to_owner_sender_and_generic_provider_kind() {
    for kind in ["telegram", "slack", "sms", "whatsapp", "custom-provider"] {
        let (mut header, mut context) = owner_fixture();
        let owner = "0x1111111111111111111111111111111111111111";
        let door = format!("0x2222222222222222222222222222222222222222#door:{kind}");
        put(&mut context, "role", text("door"));
        put(&mut context, "seat", text(&door));
        put(&mut context, "door_kind", text(kind));
        put(&mut context, "bound_sender", text("bound-human"));
        put(&mut context, "forwarded_seat", text(owner));
        put(&mut context, "expires_at", Value::Null);
        put(&mut header, "via", text(kind));
        put(&mut header, "via_sender", text("bound-human"));
        assert_eq!(verify(header.clone(), context.clone()), Ok(()));
        put(&mut header, "via_sender", text("outsider"));
        assert_eq!(
            verify(header.clone(), context.clone()),
            Err(Error::Authority)
        );
        put(&mut header, "seat", text(&door));
        put(&mut header, "role", text("external"));
        assert_eq!(verify(header.clone(), context.clone()), Ok(()));
        put(&mut header, "via", text("wrong-provider"));
        assert_eq!(
            verify(header.clone(), context.clone()),
            Err(Error::Authority)
        );
        put(&mut header, "via", Value::Null);
        put(&mut header, "via_sender", Value::Null);
        put(&mut header, "role", text("door"));
        assert_eq!(verify(header, context), Ok(()));
    }
}

#[test]
fn actor_and_builder_use_distinct_authenticated_seats() {
    for role in ["actor", "builder"] {
        let (mut header, mut context) = owner_fixture();
        let seat = if role == "builder" {
            "0x1111111111111111111111111111111111111111#builder"
        } else {
            "0x2222222222222222222222222222222222222222"
        };
        put(&mut header, "seat", text(seat));
        put(&mut header, "role", text(role));
        put(&mut context, "seat", text(seat));
        put(&mut context, "role", text(role));
        if role == "actor" {
            put(&mut context, "expires_at", Value::Null);
        }
        assert_eq!(verify(header, context), Ok(()));
    }
}
