use super::*;

/// Verify an actor delegation against independently authenticated current state.
/// `trusted_context` is canonical CBOR with exactly:
/// `{chain_id, actor, controller:bstr20, certificate_commitment:bstr32,
/// authorization_generation, room, room_owner:bstr20, membership_generation,
/// key_generation, now_ms}`. The actor/controller/commitment/generation must be
/// extracted from a verified finalized proof for the expected actor and logical
/// key; the room fields must come from current room-service state. They are NEVER
/// accepted from the enrollment HTTP body. This function does not verify a chain
/// proof or establish its freshness. It performs no network or consensus I/O.
///
/// Returns CBOR `[identity_id:bstr, seat:text, public_key:bstr,
/// append_context:bstr, from_gen:uint]`. Membership is separate from invocation
/// authority. The service must enforce from_gen on history/key access and install
/// only while the independently checked state is still current.
pub fn verify_actor_membership(
    identity: &[u8],
    identity_signature: &[u8],
    membership: &[u8],
    membership_signature: &[u8],
    trusted_context: &[u8],
) -> Result<Vec<u8>> {
    let trusted = Fields::new(
        canonical::decode(trusted_context)?,
        &[
            "chain_id",
            "actor",
            "controller",
            "certificate_commitment",
            "authorization_generation",
            "room",
            "room_owner",
            "membership_generation",
            "key_generation",
            "now_ms",
        ],
    )?;
    let actor = trusted.text("actor")?;
    let actor_address = address(&actor)?;
    if actor != format!("0x{}", hex::encode(actor_address)) {
        return Err(Error::Authority);
    }
    let identity_cert = Certificate::decode(IDENTITY, identity)?;
    let membership_cert = Certificate::decode(MEMBERSHIP, membership)?;
    let identity_id = certificate_id(IDENTITY, identity)?;
    let commitment = trusted.bytes::<32>("certificate_commitment")?;
    let from_gen = membership_cert.fields.uint("from_gen")?;
    let key_gen = trusted.uint("key_generation")?;
    if identity_cert.fields.text("address")? != actor
        || identity_cert.fields.text("role")? != "actor"
        || membership_cert.fields.text("seat")? != actor
        || membership_cert.fields.text("signer_kind")? != "wallet"
        || membership_cert.fields.nullable_text("door_kind")?.is_some()
        || membership_cert
            .fields
            .nullable_text("bound_sender")?
            .is_some()
        || from_gen > key_gen
        || identity_id.as_slice() != commitment
    {
        return Err(Error::Authority);
    }
    let make_context = |wallet: [u8; 20], generation: u64| {
        canonical::encode(Value::Map(vec![
            (
                Value::Text("chain_id".into()),
                trusted.uint("chain_id")?.into(),
            ),
            (
                Value::Text("room".into()),
                Value::Text(trusted.text("room")?),
            ),
            (Value::Text("gen".into()), generation.into()),
            (Value::Text("now_ms".into()), trusted.uint("now_ms")?.into()),
            (
                Value::Text("wallet_address".into()),
                Value::Bytes(wallet.to_vec()),
            ),
            (Value::Text("admin_key".into()), Value::Null),
        ]))
    };
    verify(
        IDENTITY,
        identity,
        identity_signature,
        &identity_id,
        &make_context(
            trusted.bytes("controller")?,
            trusted.uint("authorization_generation")?,
        )?,
    )?;
    verify(
        MEMBERSHIP,
        membership,
        membership_signature,
        &certificate_id(MEMBERSHIP, membership)?,
        &make_context(
            trusted.bytes("room_owner")?,
            trusted.uint("membership_generation")?,
        )?,
    )?;
    let public = identity_cert.fields.bytes::<32>("pubkey")?;
    let context = canonical::encode(Value::Map(vec![
        (
            Value::Text("chain_id".into()),
            trusted.uint("chain_id")?.into(),
        ),
        (
            Value::Text("room".into()),
            Value::Text(trusted.text("room")?),
        ),
        (Value::Text("gen".into()), key_gen.into()),
        (Value::Text("seat".into()), Value::Text(actor.clone())),
        (Value::Text("role".into()), Value::Text("actor".into())),
        (
            Value::Text("cert".into()),
            Value::Text(hex::encode(&identity_id)),
        ),
        (
            Value::Text("public_key".into()),
            Value::Bytes(public.to_vec()),
        ),
        (
            Value::Text("rights".into()),
            Value::Array(
                membership_cert
                    .fields
                    .strings("rights")?
                    .into_iter()
                    .map(Value::Text)
                    .collect(),
            ),
        ),
        (
            Value::Text("expires_at".into()),
            membership_cert.expiry.map_or(Value::Null, |v| v.into()),
        ),
        (Value::Text("door_kind".into()), Value::Null),
        (Value::Text("bound_sender".into()), Value::Null),
        (Value::Text("forwarded_seat".into()), Value::Null),
    ]))?;
    canonical::encode(Value::Array(vec![
        Value::Bytes(identity_id),
        Value::Text(actor),
        Value::Bytes(public.to_vec()),
        Value::Bytes(context),
        from_gen.into(),
    ]))
}
