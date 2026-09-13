use super::*;

/// Authenticate a wallet-issued builder identity and room-owner membership.
/// Trusted CBOR fields: chain_id, room, room_owner:bstr20,
/// membership_generation, key_generation, now_ms. All come from current
/// service-owned state, never the request. There is no delegated issuer here.
/// Returns CBOR [identity_id:bstr, seat:text, public_key:bstr,
/// append_context:bstr, from_gen:uint]. This grants no compute authority.
pub fn verify_builder_membership(
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
            "room",
            "room_owner",
            "membership_generation",
            "key_generation",
            "now_ms",
        ],
    )?;
    let owner = trusted.bytes::<20>("room_owner")?;
    let owner_address = format!("0x{}", hex::encode(owner));
    let seat = format!("{owner_address}#builder");
    let identity_cert = Certificate::decode(IDENTITY, identity)?;
    let member = Certificate::decode(MEMBERSHIP, membership)?;
    let from_gen = member.fields.uint("from_gen")?;
    if identity_cert.fields.text("role")? != "builder"
        || identity_cert.fields.text("address")? != owner_address
        || member.fields.text("seat")? != seat
        || member.fields.text("signer_kind")? != "wallet"
        || member.fields.nullable_text("door_kind")?.is_some()
        || member.fields.nullable_text("bound_sender")?.is_some()
        || member.fields.strings("rights")? != ["read", "write"]
        || from_gen > trusted.uint("key_generation")?
    {
        return Err(Error::Authority);
    }
    let context = canonical::encode(Value::Map(vec![
        (
            Value::Text("chain_id".into()),
            trusted.uint("chain_id")?.into(),
        ),
        (
            Value::Text("room".into()),
            Value::Text(trusted.text("room")?),
        ),
        (
            Value::Text("gen".into()),
            trusted.uint("membership_generation")?.into(),
        ),
        (Value::Text("now_ms".into()), trusted.uint("now_ms")?.into()),
        (
            Value::Text("wallet_address".into()),
            Value::Bytes(owner.to_vec()),
        ),
        (Value::Text("admin_key".into()), Value::Null),
    ]))?;
    let id = certificate_id(IDENTITY, identity)?;
    verify(IDENTITY, identity, identity_signature, &id, &context)?;
    verify(
        MEMBERSHIP,
        membership,
        membership_signature,
        &certificate_id(MEMBERSHIP, membership)?,
        &context,
    )?;
    let public = identity_cert.fields.bytes::<32>("pubkey")?;
    let expiry = identity_cert.expiry.ok_or(Error::Expiry)?;
    let expiry = member.expiry.map_or(expiry, |e| e.min(expiry));
    let append = canonical::encode(Value::Map(vec![
        (
            Value::Text("chain_id".into()),
            trusted.uint("chain_id")?.into(),
        ),
        (
            Value::Text("room".into()),
            Value::Text(trusted.text("room")?),
        ),
        (
            Value::Text("gen".into()),
            trusted.uint("key_generation")?.into(),
        ),
        (Value::Text("seat".into()), Value::Text(seat.clone())),
        (Value::Text("role".into()), Value::Text("builder".into())),
        (Value::Text("cert".into()), Value::Text(hex::encode(&id))),
        (
            Value::Text("public_key".into()),
            Value::Bytes(public.to_vec()),
        ),
        (
            Value::Text("rights".into()),
            Value::Array(vec![
                Value::Text("read".into()),
                Value::Text("write".into()),
            ]),
        ),
        (Value::Text("expires_at".into()), expiry.into()),
        (Value::Text("door_kind".into()), Value::Null),
        (Value::Text("bound_sender".into()), Value::Null),
        (Value::Text("forwarded_seat".into()), Value::Null),
    ]))?;
    canonical::encode(Value::Array(vec![
        Value::Bytes(id),
        Value::Text(seat),
        Value::Bytes(public.to_vec()),
        Value::Bytes(append),
        from_gen.into(),
    ]))
}
