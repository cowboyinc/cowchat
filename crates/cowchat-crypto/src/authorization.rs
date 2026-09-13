//! Bind an encrypted record to independently authenticated, current membership.
//! This does not resolve Cowboy identity or issue membership. The service must
//! build the context from its verified identity/controller and membership state,
//! never accept it as a client-supplied proof. A revoked seat has no context.
use crate::{canonical, envelope, fields::Fields, Error, Result};

/// Verify a member append without decrypting the body or accessing consensus.
///
/// `trusted_context` is canonical CBOR with these fields:
/// `{chain_id, room, gen, seat, role, cert, public_key: bstr32, rights: [text],
/// expires_at: null|uint, door_kind: null|text, bound_sender: null|text,
/// forwarded_seat: null|text}`. Expiry is the earliest applicable credential or
/// membership expiry. Actor/controller commitment and credential-generation
/// checks must have succeeded before the service constructs this context.
///
/// Success authorizes this record's append only. In particular, mentions do
/// not grant permission to wake a builder or incur charges. The service must
/// revalidate current membership in the transaction that commits the append.
pub fn verify_member_record(
    header: &[u8],
    body: &str,
    signature: &[u8],
    trusted_context: &[u8],
    now_ms: u64,
) -> Result<()> {
    let context = context(trusted_context)?;
    let public_key = context.bytes::<32>("public_key")?;
    envelope::verify_record(header, body, &public_key, signature)?;
    let record = Fields::new(canonical::decode(header)?, envelope::HEADER_FIELDS)?;
    if record.uint("chain_id")? != context.uint("chain_id")?
        || record.text("room")? != context.text("room")?
        || record.uint("gen")? != context.uint("gen")?
        || record.text("cert")? != context.text("cert")?
    {
        return Err(Error::Scope);
    }
    check_access(&context, "write", now_ms)?;
    if record.text("class")? == "system" {
        return Err(Error::Authority);
    }
    let seat = record.text("seat")?;
    let role = record.text("role")?;
    let principal = context.text("seat")?;
    let principal_role = context.text("role")?;
    let via = record.nullable_text("via")?;
    let sender = record.nullable_text("via_sender")?;
    let door = context.nullable_text("door_kind")?;
    let bound_sender = context.nullable_text("bound_sender")?;
    let forwarded_seat = context.nullable_text("forwarded_seat")?;
    match principal_role.as_str() {
        "owner" | "builder" | "actor" => {
            if seat != principal
                || role != principal_role
                || sender.is_some()
                || door.is_some()
                || bound_sender.is_some()
                || forwarded_seat.is_some()
            {
                return Err(Error::Authority);
            }
        }
        "door" => {
            let (Some(door), Some(bound), Some(owner)) = (door, bound_sender, forwarded_seat)
            else {
                return Err(Error::Authority);
            };
            let own = seat == principal && role == "door" && via.is_none() && sender.is_none();
            let forwarded = seat == owner
                && role == "owner"
                && via.as_ref() == Some(&door)
                && sender.as_ref() == Some(&bound);
            let external = seat == principal
                && role == "external"
                && via.as_ref() == Some(&door)
                && sender.as_ref().is_some_and(|sender| sender != &bound);
            if !own && !forwarded && !external {
                return Err(Error::Authority);
            }
        }
        _ => return Err(Error::Authority),
    }
    Ok(())
}

fn context(trusted_context: &[u8]) -> Result<Fields> {
    Fields::new(
        canonical::decode(trusted_context)?,
        &[
            "chain_id",
            "room",
            "gen",
            "seat",
            "role",
            "cert",
            "public_key",
            "rights",
            "expires_at",
            "door_kind",
            "bound_sender",
            "forwarded_seat",
        ],
    )
}

fn check_access(context: &Fields, required: &str, now_ms: u64) -> Result<()> {
    if !["read", "write", "manage"].contains(&required) {
        return Err(Error::Authority);
    }
    if context
        .nullable_uint("expires_at")?
        .is_some_and(|expiry| now_ms > expiry)
    {
        return Err(Error::Expiry);
    }
    let rights = context.strings("rights")?;
    if !rights.iter().any(|right| right == required)
        || rights
            .iter()
            .any(|right| !["manage", "read", "write"].contains(&right.as_str()))
        || rights.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Error::Authority);
    }
    Ok(())
}

/// Check a service operation against already-authenticated, current membership.
/// The caller still binds the signed request and rechecks live room generation
/// in its transaction. This never grants invocation/spend authority.
pub fn verify_member_access(trusted_context: &[u8], required: &str, now_ms: u64) -> Result<()> {
    check_access(&context(trusted_context)?, required, now_ms)
}
