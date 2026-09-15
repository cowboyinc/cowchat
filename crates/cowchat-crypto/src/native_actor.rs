//! Strict Cowchat v3 envelope profile for chain-authorized source seats.
//!
//! The caller must obtain every expected field and the record-signing key from
//! one finalized room-authority proof. Actor and human seats use the same
//! signature boundary because chain seat admission has already verified their
//! controller authorization. Gateway ingress requires a separate attribution
//! profile and is refused here.
use crate::{canonical, envelope, fields::Fields, Error, Result};
use ciborium::value::Value;
use ed25519_dalek::SigningKey;
use std::io::Cursor;

const HEADER_FIELDS: &[&str] = &[
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

/// Maximum accepted outer tuple size. The encrypted body and canonical header
/// retain their own tighter bounds.
pub const MAX_SEALED_RECORD_BYTES: usize = envelope::MAX_BODY_BYTES + canonical::MAX_BYTES + 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceSeatKindV1 {
    Actor,
    Human,
    Gateway,
}

impl SourceSeatKindV1 {
    fn header_role(self) -> Result<&'static str> {
        match self {
            Self::Actor => Ok("actor"),
            Self::Human => Ok("human"),
            Self::Gateway => Err(Error::Scope),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedSourceSeatRecordV1 {
    pub chain_id: u64,
    pub room_id: [u8; 32],
    pub source_seat_id: [u8; 32],
    pub source_seat_kind: SourceSeatKindV1,
    pub source_key_binding_commitment: [u8; 32],
    pub key_generation: u64,
    /// A routed actor message must authenticate the target mention in the
    /// signed header before the body is decrypted.
    pub target_seat_id: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenedSourceSeatRecordV1 {
    pub message_id: [u8; 32],
    pub reply_to: Option<[u8; 32]>,
    pub mentions: Vec<[u8; 32]>,
    pub plaintext: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorReplyHeaderV1 {
    pub chain_id: u64,
    pub room_id: [u8; 32],
    pub seat_id: [u8; 32],
    pub key_binding_commitment: [u8; 32],
    pub key_generation: u64,
    pub message_id: [u8; 32],
    pub reply_to: [u8; 32],
}

struct SealedRecord {
    header: Vec<u8>,
    body: String,
    signature: Vec<u8>,
}

struct CheckedSourceSeatHeader {
    message_id: [u8; 32],
    reply_to: Option<[u8; 32]>,
    mentions: Vec<[u8; 32]>,
}

fn nonzero_32(value: &[u8; 32]) -> Result<()> {
    if *value == [0; 32] {
        Err(Error::Schema)
    } else {
        Ok(())
    }
}

fn hex32(value: &[u8; 32]) -> String {
    hex::encode(value)
}

fn parse_hex32(value: String) -> Result<[u8; 32]> {
    if value.len() != 64
        || value
            .bytes()
            .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
    {
        return Err(Error::Schema);
    }
    hex::decode(value)
        .map_err(|_| Error::Schema)?
        .try_into()
        .map_err(|_| Error::Schema)
}

fn parse_sealed_record(raw: &[u8]) -> Result<SealedRecord> {
    if raw.len() > MAX_SEALED_RECORD_BYTES {
        return Err(Error::Limit);
    }
    let mut cursor = Cursor::new(raw);
    let value: Value = ciborium::from_reader(&mut cursor).map_err(|_| Error::Encoding)?;
    if cursor.position() != raw.len() as u64 {
        return Err(Error::Encoding);
    }
    let Value::Array(values) = value else {
        return Err(Error::Schema);
    };
    let [Value::Bytes(header), Value::Text(body), Value::Bytes(signature)] = values.as_slice()
    else {
        return Err(Error::Schema);
    };
    if body.len() > envelope::MAX_BODY_BYTES || signature.len() != 64 {
        return Err(Error::Limit);
    }
    canonical::validate(header)?;
    Ok(SealedRecord {
        header: header.clone(),
        body: body.clone(),
        signature: signature.clone(),
    })
}

fn parse_header(header: &[u8]) -> Result<Fields> {
    Fields::new(canonical::decode(header)?, HEADER_FIELDS)
}

fn check_source_seat_record_header(
    header: &[u8],
    expected: &ExpectedSourceSeatRecordV1,
) -> Result<CheckedSourceSeatHeader> {
    if expected.chain_id == 0 || expected.key_generation == 0 {
        return Err(Error::Schema);
    }
    for value in [
        &expected.room_id,
        &expected.source_seat_id,
        &expected.source_key_binding_commitment,
        &expected.target_seat_id,
    ] {
        nonzero_32(value)?;
    }

    let expected_role = expected.source_seat_kind.header_role()?;
    let fields = parse_header(header)?;
    let mentions = fields
        .strings("mentions")?
        .into_iter()
        .map(parse_hex32)
        .collect::<Result<Vec<_>>>()?;
    let message_id = parse_hex32(fields.text("message_id")?)?;
    let reply_to = fields
        .nullable_text("reply_to")?
        .map(parse_hex32)
        .transpose()?;
    let expected_room = hex32(&expected.room_id);
    let expected_seat = hex32(&expected.source_seat_id);
    let expected_cert = hex32(&expected.source_key_binding_commitment);

    if fields.uint("v")? != 3
        || fields.uint("chain_id")? != expected.chain_id
        || fields.text("room")? != expected_room
        || fields.text("seat")? != expected_seat
        || fields.text("role")? != expected_role
        || fields.nullable_text("via")?.is_some()
        || fields.nullable_text("via_sender")?.is_some()
        || fields.text("class")? != "message"
        || !matches!(fields.text("wake_hint")?.as_str(), "normal" | "urgent")
        || fields.uint("gen")? != expected.key_generation
        || fields.text("cert")? != expected_cert
        || !mentions.contains(&expected.target_seat_id)
    {
        return Err(Error::Scope);
    }
    Ok(CheckedSourceSeatHeader {
        message_id,
        reply_to,
        mentions,
    })
}

/// Verify the source seat signature, principal-kind role and every
/// proof-derived identity coordinate before releasing plaintext to the caller.
pub fn open_source_seat_record_v1(
    sealed_record: &[u8],
    expected: &ExpectedSourceSeatRecordV1,
    source_record_signing_key: &[u8; 32],
    generation_secret: &[u8; 32],
) -> Result<OpenedSourceSeatRecordV1> {
    // Gateway records require provider attribution fields that this direct
    // seat profile does not understand. Refuse them before parsing or opening
    // any caller-controlled ciphertext.
    expected.source_seat_kind.header_role()?;
    nonzero_32(source_record_signing_key)?;
    let sealed = parse_sealed_record(sealed_record)?;
    envelope::verify_record(
        &sealed.header,
        &sealed.body,
        source_record_signing_key,
        &sealed.signature,
    )?;
    let checked = check_source_seat_record_header(&sealed.header, expected)?;
    let plaintext = envelope::open(
        &sealed.header,
        &sealed.body,
        source_record_signing_key,
        &sealed.signature,
        generation_secret,
    )?;
    Ok(OpenedSourceSeatRecordV1 {
        message_id: checked.message_id,
        reply_to: checked.reply_to,
        mentions: checked.mentions,
        plaintext,
    })
}

/// Read the signed public message id without decrypting the room body.
///
/// Recovery code uses this to find a previously appended sealed reply after an
/// ambiguous transport result. The id is returned only after the record
/// signature verifies under the finalized seat key supplied by the caller.
pub fn authenticated_record_message_id_v1(
    sealed_record: &[u8],
    record_signing_key: &[u8; 32],
) -> Result<[u8; 32]> {
    nonzero_32(record_signing_key)?;
    let sealed = parse_sealed_record(sealed_record)?;
    envelope::verify_record(
        &sealed.header,
        &sealed.body,
        record_signing_key,
        &sealed.signature,
    )?;
    parse_hex32(parse_header(&sealed.header)?.text("message_id")?)
}

/// Build the one canonical direct actor-reply header. Native replies carry no
/// gateway attribution and do not wake another seat implicitly.
pub fn actor_reply_header_v1(reply: &ActorReplyHeaderV1) -> Result<Vec<u8>> {
    if reply.chain_id == 0 || reply.key_generation == 0 {
        return Err(Error::Schema);
    }
    for value in [
        &reply.room_id,
        &reply.seat_id,
        &reply.key_binding_commitment,
        &reply.message_id,
        &reply.reply_to,
    ] {
        nonzero_32(value)?;
    }
    canonical::encode(Value::Map(vec![
        (Value::Text("v".into()), Value::Integer(3.into())),
        (
            Value::Text("message_id".into()),
            Value::Text(hex32(&reply.message_id)),
        ),
        (
            Value::Text("chain_id".into()),
            Value::Integer(reply.chain_id.into()),
        ),
        (
            Value::Text("room".into()),
            Value::Text(hex32(&reply.room_id)),
        ),
        (
            Value::Text("seat".into()),
            Value::Text(hex32(&reply.seat_id)),
        ),
        (Value::Text("role".into()), Value::Text("actor".into())),
        (Value::Text("via".into()), Value::Null),
        (Value::Text("via_sender".into()), Value::Null),
        (Value::Text("class".into()), Value::Text("message".into())),
        (
            Value::Text("reply_to".into()),
            Value::Text(hex32(&reply.reply_to)),
        ),
        (Value::Text("mentions".into()), Value::Array(Vec::new())),
        (Value::Text("wake_hint".into()), Value::Text("none".into())),
        (
            Value::Text("gen".into()),
            Value::Integer(reply.key_generation.into()),
        ),
        (
            Value::Text("cert".into()),
            Value::Text(hex32(&reply.key_binding_commitment)),
        ),
        (
            Value::Text("nonce".into()),
            Value::Text("AAAAAAAAAAAAAAAA".into()),
        ),
    ]))
}

/// Seal a canonical direct actor reply and require the signing seed to match
/// the finalized seat record key supplied by the caller.
pub fn seal_actor_reply_v1(
    reply: &ActorReplyHeaderV1,
    generation_secret: &[u8; 32],
    plaintext: &[u8],
    signing_seed: &[u8; 32],
    expected_record_signing_key: &[u8; 32],
) -> Result<Vec<u8>> {
    let actual = SigningKey::from_bytes(signing_seed)
        .verifying_key()
        .to_bytes();
    if actual != *expected_record_signing_key {
        return Err(Error::Authority);
    }
    envelope::seal(
        &actor_reply_header_v1(reply)?,
        generation_secret,
        plaintext,
        signing_seed,
    )
}

/// Validate an actor reply against its exact protocol-derived identifiers and
/// signing key, then decrypt it. This is also a caller-seam check for retries.
pub fn open_actor_reply_v1(
    sealed_record: &[u8],
    expected: &ActorReplyHeaderV1,
    record_signing_key: &[u8; 32],
    generation_secret: &[u8; 32],
) -> Result<Vec<u8>> {
    let sealed = parse_sealed_record(sealed_record)?;
    envelope::verify_record(
        &sealed.header,
        &sealed.body,
        record_signing_key,
        &sealed.signature,
    )?;
    let expected_header = actor_reply_header_v1(expected)?;
    let actual = parse_header(&sealed.header)?;
    let canonical = parse_header(&expected_header)?;
    for key in HEADER_FIELDS.iter().copied().filter(|key| *key != "nonce") {
        if actual.get(key)? != canonical.get(key)? {
            return Err(Error::Scope);
        }
    }
    envelope::open(
        &sealed.header,
        &sealed.body,
        record_signing_key,
        &sealed.signature,
        generation_secret,
    )
}
