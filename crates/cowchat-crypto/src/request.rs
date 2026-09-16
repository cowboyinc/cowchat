//! Signed request projection. The server must bind these fields to the raw
//! received method/target/body and atomically claim the returned nonce token.
use crate::{canonical, signatures, Error, Result};
use ciborium::value::Value;
use sha2::{Digest, Sha256};

pub const DOMAIN: &[u8] = b"cowchat/v3/request";
pub const WINDOW_MS: u64 = 300_000;

struct Request {
    method: String,
    target: String,
    body_hash: [u8; 32],
    stamp: u64,
    nonce: [u8; 16],
}
impl Request {
    fn decode(raw: &[u8]) -> Result<Self> {
        let Value::Array(fields) = canonical::decode(raw)? else {
            return Err(Error::Schema);
        };
        let [Value::Text(method), Value::Text(target), Value::Bytes(hash), Value::Integer(stamp), Value::Bytes(nonce)] =
            fields.as_slice()
        else {
            return Err(Error::Schema);
        };
        if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].contains(&method.as_str())
            || !target.starts_with('/')
            || target.contains('#')
            || target.bytes().any(|b| !(33..=126).contains(&b))
        {
            return Err(Error::Schema);
        }
        Ok(Self {
            method: method.clone(),
            target: target.clone(),
            body_hash: hash.as_slice().try_into().map_err(|_| Error::Schema)?,
            stamp: u64::try_from(*stamp).map_err(|_| Error::Schema)?,
            nonce: nonce.as_slice().try_into().map_err(|_| Error::Schema)?,
        })
    }
}

pub fn signing_bytes(projection: &[u8]) -> Result<Vec<u8>> {
    Request::decode(projection)?;
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(projection);
    Ok(bytes)
}
pub fn sign(projection: &[u8], seed: &[u8]) -> Result<Vec<u8>> {
    signatures::sign_ed25519(seed, &signing_bytes(projection)?)
}

/// Verifies only the signed projection and time. Returns CBOR
/// `[public_key_bstr, nonce_bstr, retain_through_ms]`. The server MUST make a
/// durable, atomic insert-if-absent keyed by (public key, nonce) before acting.
/// This pure function never claims replay protection or current membership.
pub fn verify_projection(
    projection: &[u8],
    public_key: &[u8],
    signature: &[u8],
    now_ms: u64,
) -> Result<Vec<u8>> {
    let request = Request::decode(projection)?;
    signatures::verify_ed25519(public_key, signature, &signing_bytes(projection)?)?;
    if request.stamp.abs_diff(now_ms) > WINDOW_MS {
        return Err(Error::Timestamp);
    }
    canonical::encode(Value::Array(vec![
        Value::Bytes(public_key.to_vec()),
        Value::Bytes(request.nonce.to_vec()),
        Value::Integer(request.stamp.saturating_add(WINDOW_MS).into()),
    ]))
}

/// Caller seam: also bind the signature to what the HTTP server actually
/// received. Pass the raw origin-form request target, including untouched query.
pub fn verify_received(
    projection: &[u8],
    public_key: &[u8],
    signature: &[u8],
    now_ms: u64,
    method: &str,
    raw_target: &str,
    body: &[u8],
) -> Result<Vec<u8>> {
    let request = Request::decode(projection)?;
    if request.method != method
        || request.target != raw_target
        || request.body_hash.as_slice() != Sha256::digest(body).as_slice()
    {
        return Err(Error::Scope);
    }
    verify_projection(projection, public_key, signature, now_ms)
}
