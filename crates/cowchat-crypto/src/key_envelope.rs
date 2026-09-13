//! Portable publisher authentication over scoped HPKE ciphertext. This module
//! never releases or unwraps secrets. Callers compare scope with independently
//! authenticated current membership BEFORE using the existing HPKE unwrap API.
use crate::{canonical, fields::Fields, signatures, Error, Result};
use ciborium::value::Value;

/// Canonical CBOR scope fields (all required). Generation values are uints;
/// recipient_key is bstr32, and all other fields except v are nonempty text.
/// `purpose` is fixed to room-generation-secret; v is 1 for this envelope type.
fn signing_bytes(scope: &[u8], wrapped: &[u8]) -> Result<Vec<u8>> {
    if scope.len() > 4096 || wrapped.len() > 128 {
        return Err(Error::Limit);
    }
    let fields = Fields::new(
        canonical::decode(scope)?,
        &[
            "v",
            "chain_id",
            "room",
            "auth_generation",
            "key_generation",
            "transport_generation",
            "recipient_seat",
            "recipient_cert",
            "recipient_key",
            "publisher_cert",
            "purpose",
        ],
    )?;
    if fields.uint("v")? != 1 || fields.text("purpose")? != "room-generation-secret" {
        return Err(Error::Schema);
    }
    for name in [
        "chain_id",
        "auth_generation",
        "key_generation",
        "transport_generation",
    ] {
        fields.uint(name)?;
    }
    for name in ["room", "recipient_seat", "recipient_cert", "publisher_cert"] {
        fields.text(name)?;
    }
    fields.bytes::<32>("recipient_key")?;
    let Value::Array(parts) = canonical::decode(wrapped)? else {
        return Err(Error::Schema);
    };
    let [Value::Bytes(enc), Value::Bytes(ct)] = parts.as_slice() else {
        return Err(Error::Schema);
    };
    if enc.len() != 32 || ct.len() != 48 {
        return Err(Error::Schema);
    }
    let mut bytes = b"cowchat/v3/room-key-envelope/v1".to_vec();
    bytes.extend(canonical::encode(Value::Array(vec![
        Value::Bytes(scope.to_vec()),
        Value::Bytes(wrapped.to_vec()),
    ]))?);
    Ok(bytes)
}

/// Sign a wrap created by keys::wrap_room_key. Borrow the publisher seed only
/// for this call. Exact retries retain scope, wrap and signature unchanged.
pub fn sign(scope: &[u8], wrapped: &[u8], publisher_seed: &[u8]) -> Result<Vec<u8>> {
    signatures::sign_ed25519(publisher_seed, &signing_bytes(scope, wrapped)?)
}

/// Verifies bytes only, NOT authorization. The public key and expected scope
/// must be resolved independently of the envelope being verified.
pub fn verify(
    scope: &[u8],
    wrapped: &[u8],
    signature: &[u8],
    publisher_public: &[u8],
) -> Result<()> {
    signatures::verify_ed25519(publisher_public, signature, &signing_bytes(scope, wrapped)?)
}
