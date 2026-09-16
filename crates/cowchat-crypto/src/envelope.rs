//! Record cryptography. Signatures alone do not authorize seats, door forwarding,
//! system records, or invocations; those are checks on authenticated service state.
use crate::{canonical, fields::Fields, keys, signatures, Error, Result};
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use ciborium::value::Value;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MAX_BODY_BYTES: usize = 1_048_576;
const DOMAIN: &[u8] = b"cowchat/v3/envelope";
const FIELDS: &[&str] = &[
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

struct Header {
    room: String,
    nonce: [u8; 12],
}
impl Header {
    fn decode(raw: &[u8]) -> Result<Self> {
        let f = Fields::new(canonical::decode(raw)?, FIELDS)?;
        if f.uint("v")? != 3 {
            return Err(Error::Schema);
        }
        f.uint("chain_id")?;
        f.uint("gen")?;
        for key in ["message_id", "seat", "cert"] {
            f.text(key)?;
        }
        f.nullable_text("reply_to")?;
        f.strings("mentions")?;
        if !["owner", "builder", "human", "actor", "door", "external"]
            .contains(&f.text("role")?.as_str())
        {
            return Err(Error::Schema);
        }
        let class = f.text("class")?;
        let hint = f.text("wake_hint")?;
        if !["message", "thinking", "system"].contains(&class.as_str())
            || !["none", "normal", "urgent"].contains(&hint.as_str())
            || (class == "thinking" && hint != "none")
        {
            return Err(Error::Schema);
        }
        if f.nullable_text("via")?.is_none() && f.nullable_text("via_sender")?.is_some() {
            return Err(Error::Schema);
        }
        f.nullable_text("via_sender")?;
        let nonce = B64
            .decode(f.text("nonce")?)
            .map_err(|_| Error::Schema)?
            .try_into()
            .map_err(|_| Error::Schema)?;
        Ok(Self {
            room: f.text("room")?,
            nonce,
        })
    }
}

pub fn validate_header(header: &[u8]) -> Result<()> {
    Header::decode(header).map(|_| ())
}

pub fn signing_bytes(header: &[u8], body: &str) -> Result<Vec<u8>> {
    Header::decode(header)?;
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::Limit);
    }
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(&Sha256::digest(body.as_bytes()));
    Ok(bytes)
}

/// Verify signature and framing, without decrypting or granting any authority.
pub fn verify_record(header: &[u8], body: &str, public_key: &[u8], signature: &[u8]) -> Result<()> {
    let parsed = Header::decode(header)?;
    signatures::verify_ed25519(public_key, signature, &signing_bytes(header, body)?)?;
    let raw = B64
        .decode(body.strip_prefix("cow1:").ok_or(Error::Schema)?)
        .map_err(|_| Error::Schema)?;
    if raw.len() < 28 || raw[..12] != parsed.nonce {
        return Err(Error::Nonce);
    }
    Ok(())
}

/// Decrypt only after verifying against a key authenticated by the caller.
pub fn open(
    header: &[u8],
    body: &str,
    public_key: &[u8],
    signature: &[u8],
    generation_secret: &[u8],
) -> Result<Vec<u8>> {
    verify_record(header, body, public_key, signature)?;
    let parsed = Header::decode(header)?;
    if generation_secret.len() != 32 {
        return Err(Error::Schema);
    }
    let key = Zeroizing::new(keys::derive_room_key(generation_secret, &parsed.room));
    let raw = B64.decode(&body[5..]).map_err(|_| Error::Schema)?;
    ChaCha20Poly1305::new_from_slice(key.as_slice())
        .map_err(|_| Error::Schema)?
        .decrypt(
            Nonce::from_slice(&parsed.nonce),
            Payload {
                msg: &raw[12..],
                aad: header,
            },
        )
        .map_err(|_| Error::Decrypt)
}

/// Seal once, persist the returned bytes before sending, and retry those bytes.
/// Replaces the input header nonce with fresh OS randomness; no API exposes
/// production encryption under a caller-selected nonce. Returns CBOR
/// `[header_cbor_bstr, cow1_body_text, signature_bstr]`.
pub fn seal(
    header: &[u8],
    generation_secret: &[u8],
    plaintext: &[u8],
    signing_seed: &[u8],
) -> Result<Vec<u8>> {
    use rand::TryRngCore;
    Header::decode(header)?;
    if generation_secret.len() != 32 || plaintext.len() > (MAX_BODY_BYTES - 128) * 3 / 4 {
        return Err(Error::Limit);
    }
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| Error::Random)?;
    let Value::Map(mut fields) = canonical::decode(header)? else {
        return Err(Error::Schema);
    };
    for (key, value) in &mut fields {
        if key == &Value::Text("nonce".into()) {
            *value = Value::Text(B64.encode(nonce));
        }
    }
    let header = canonical::encode(Value::Map(fields))?;
    let parsed = Header::decode(&header)?;
    let key = Zeroizing::new(keys::derive_room_key(generation_secret, &parsed.room));
    let ct = ChaCha20Poly1305::new_from_slice(key.as_slice())
        .map_err(|_| Error::Schema)?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &header,
            },
        )
        .map_err(|_| Error::Schema)?;
    let mut raw = nonce.to_vec();
    raw.extend_from_slice(&ct);
    let body = format!("cow1:{}", B64.encode(raw));
    let sig = signatures::sign_ed25519(signing_seed, &signing_bytes(&header, &body)?)?;
    // A complete sealed record can exceed the signed-object 64KiB bound;
    // encode the outer transport tuple independently after bounding the body.
    let mut output = Vec::new();
    ciborium::into_writer(
        &Value::Array(vec![
            Value::Bytes(header),
            Value::Text(body),
            Value::Bytes(sig),
        ]),
        &mut output,
    )
    .map_err(|_| Error::Encoding)?;
    Ok(output)
}
