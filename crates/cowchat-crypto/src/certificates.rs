//! Certificate signatures and explicit trusted-context checks. The caller must
//! resolve the wallet/admin key and current generations from authenticated state.
//! No boolean from the wire can assert controller ownership or manage rights.
use crate::{canonical, fields::Fields, signatures, Error, Result};
use ciborium::value::Value;
use sha2::{Digest, Sha256};

mod actor;
mod builder;
pub use actor::verify_actor_membership;
pub use builder::verify_builder_membership;

pub const IDENTITY: u32 = 1;
pub const MEMBERSHIP: u32 = 2;
pub const INVOCATION: u32 = 3;

fn domain(kind: u32) -> Result<&'static [u8]> {
    match kind {
        IDENTITY => Ok(b"cowchat/v3/cert/identity"),
        MEMBERSHIP => Ok(b"cowchat/v3/cert/membership"),
        INVOCATION => Ok(b"cowchat/v3/cert/invocation"),
        _ => Err(Error::Schema),
    }
}

enum Signer {
    Wallet,
    Admin([u8; 32]),
}
struct Certificate {
    fields: Fields,
    chain: u64,
    generation: u64,
    expiry: Option<u64>,
    signer: Signer,
}
impl Certificate {
    fn decode(kind: u32, raw: &[u8]) -> Result<Self> {
        let names: &[&str] = match kind {
            IDENTITY => &[
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
            MEMBERSHIP => &[
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
            INVOCATION => &[
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
            _ => return Err(Error::Schema),
        };
        let fields = Fields::new(canonical::decode(raw)?, names)?;
        if fields.uint("v")? != 3 {
            return Err(Error::Schema);
        }
        let chain = fields.uint("chain_id")?;
        let generation = fields.uint("gen")?;
        let expiry = fields.nullable_uint("expires_at")?;
        let mut signer = Signer::Wallet;
        match kind {
            IDENTITY => {
                address(&fields.text("address")?)?;
                fields.bytes::<32>("pubkey")?;
                fields.bytes::<32>("enc_pubkey")?;
                if fields.text("aud")? != "cowchat" {
                    return Err(Error::Scope);
                }
                let role = fields.text("role")?;
                if !["owner", "builder", "actor", "door"].contains(&role.as_str())
                    || (matches!(role.as_str(), "owner" | "builder") && expiry.is_none())
                    || (matches!(role.as_str(), "actor" | "door") && expiry.is_some())
                {
                    return Err(Error::Schema);
                }
            }
            MEMBERSHIP => {
                fields.text("room")?;
                fields.text("seat")?;
                fields.uint("from_gen")?;
                let rights = fields.strings("rights")?;
                if rights.is_empty()
                    || rights
                        .iter()
                        .any(|r| !["manage", "read", "write"].contains(&r.as_str()))
                    || rights.windows(2).any(|r| r[0] >= r[1])
                {
                    return Err(Error::Schema);
                }
                if fields.nullable_text("door_kind")?.is_some()
                    != fields.nullable_text("bound_sender")?.is_some()
                {
                    return Err(Error::Schema);
                }
                match fields.text("signer_kind")?.as_str() {
                    "wallet" if matches!(fields.get("signer_key")?, Value::Null) => (),
                    "admin" => signer = Signer::Admin(fields.bytes("signer_key")?),
                    _ => return Err(Error::Schema),
                }
            }
            INVOCATION => {
                fields.text("room")?;
                fields.text("grantee_seat")?;
                fields.text("target_seat")?;
                if fields.text("scope")? != "wake" {
                    return Err(Error::Schema);
                }
                let _budget_wei = u128::from_be_bytes(fields.bytes("budget")?);
            }
            _ => unreachable!(),
        }
        Ok(Self {
            fields,
            chain,
            generation,
            expiry,
            signer,
        })
    }
}

pub fn validate(kind: u32, certificate: &[u8]) -> Result<()> {
    Certificate::decode(kind, certificate).map(|_| ())
}
pub fn signing_bytes(kind: u32, certificate: &[u8]) -> Result<Vec<u8>> {
    Certificate::decode(kind, certificate)?;
    let mut bytes = domain(kind)?.to_vec();
    bytes.extend_from_slice(certificate);
    Ok(bytes)
}
pub fn certificate_id(kind: u32, certificate: &[u8]) -> Result<Vec<u8>> {
    Ok(Sha256::digest(signing_bytes(kind, certificate)?).to_vec())
}

fn address(text: &str) -> Result<[u8; 20]> {
    let hex = text.strip_prefix("0x").ok_or(Error::Schema)?;
    if hex.len() != 40 {
        return Err(Error::Schema);
    }
    hex::decode(hex)
        .map_err(|_| Error::Schema)?
        .try_into()
        .map_err(|_| Error::Schema)
}

/// Verify a certificate with trusted CBOR context:
/// `{chain_id, room, gen, now_ms, wallet_address: bstr20, admin_key: null|bstr32}`.
/// `admin_key` must come from an independently verified, current owner
/// delegation with manage rights, never from the certificate being checked.
/// Actor controller/commitment, seat rights and revocation are caller checks.
pub fn verify(
    kind: u32,
    certificate: &[u8],
    signature: &[u8],
    expected_id: &[u8],
    trusted_context: &[u8],
) -> Result<()> {
    let cert = Certificate::decode(kind, certificate)?;
    let context = Fields::new(
        canonical::decode(trusted_context)?,
        &[
            "chain_id",
            "room",
            "gen",
            "now_ms",
            "wallet_address",
            "admin_key",
        ],
    )?;
    let owner = context.bytes::<20>("wallet_address")?;
    let admin = match context.get("admin_key")? {
        Value::Null => None,
        _ => Some(context.bytes::<32>("admin_key")?),
    };
    if cert.chain != context.uint("chain_id")?
        || cert.generation != context.uint("gen")?
        || (kind != IDENTITY && cert.fields.text("room")? != context.text("room")?)
    {
        return Err(Error::Scope);
    }
    let now = context.uint("now_ms")?;
    if cert.expiry.is_some_and(|e| e < now) {
        return Err(Error::Expiry);
    }
    let signed = signing_bytes(kind, certificate)?;
    if expected_id.len() != 32 || Sha256::digest(&signed).as_slice() != expected_id {
        return Err(Error::CertificateId);
    }
    match cert.signer {
        Signer::Wallet => {
            if signatures::recover_wallet(signature, &signed)? != owner {
                return Err(Error::Authority);
            }
            if kind == IDENTITY
                && matches!(cert.fields.text("role")?.as_str(), "owner" | "builder")
                && address(&cert.fields.text("address")?)? != owner
            {
                return Err(Error::Authority);
            }
        }
        Signer::Admin(key) => {
            signatures::verify_ed25519(&key, signature, &signed)?;
            if admin != Some(key) {
                return Err(Error::Authority);
            }
        }
    }
    Ok(())
}

/// Only delegated-admin membership is signed with Ed25519. Wallets sign the
/// Keccak digest of `signing_bytes` externally; private wallet keys never enter
/// this API.
pub fn sign_admin(certificate: &[u8], signing_seed: &[u8]) -> Result<Vec<u8>> {
    let cert = Certificate::decode(MEMBERSHIP, certificate)?;
    let Signer::Admin(key) = cert.signer else {
        return Err(Error::Schema);
    };
    let bytes = signing_bytes(MEMBERSHIP, certificate)?;
    let sig = signatures::sign_ed25519(signing_seed, &bytes)?;
    signatures::verify_ed25519(&key, &sig, &bytes)?;
    Ok(sig)
}

/// Authenticate the initial owner of a newly created room using wallet-signed
/// identity and membership certificates. No chain I/O or private keys are needed.
/// Returns CBOR `[identity_id_bstr, seat_text, public_key_bstr, append_context_bstr,
/// wallet_address_bstr]`. The service must separately authorize the existing room
/// creator, require an empty room, and atomically install this as generation zero.
/// This is an owner-only bootstrap; it cannot enroll an actor or assert control of one.
pub fn verify_initial_owner(
    room: &str,
    identity: &[u8],
    identity_signature: &[u8],
    membership: &[u8],
    membership_signature: &[u8],
    now_ms: u64,
) -> Result<Vec<u8>> {
    let identity_cert = Certificate::decode(IDENTITY, identity)?;
    let membership_cert = Certificate::decode(MEMBERSHIP, membership)?;
    let seat = identity_cert.fields.text("address")?;
    let wallet = address(&seat)?;
    // Use one spelling for seat identity, not multiple hex aliases.
    if seat != format!("0x{}", hex::encode(wallet))
        || identity_cert.fields.text("role")? != "owner"
        || identity_cert.generation != 0
        || membership_cert.generation != 0
        || membership_cert.fields.uint("from_gen")? != 0
        || membership_cert.fields.text("seat")? != seat
        || membership_cert.fields.text("signer_kind")? != "wallet"
        || membership_cert.fields.nullable_text("door_kind")?.is_some()
        || membership_cert
            .fields
            .nullable_text("bound_sender")?
            .is_some()
        || membership_cert.fields.strings("rights")? != ["manage", "read", "write"]
    {
        return Err(Error::Authority);
    }
    let trusted = canonical::encode(Value::Map(vec![
        (Value::Text("chain_id".into()), identity_cert.chain.into()),
        (Value::Text("room".into()), Value::Text(room.into())),
        (Value::Text("gen".into()), 0u64.into()),
        (Value::Text("now_ms".into()), now_ms.into()),
        (
            Value::Text("wallet_address".into()),
            Value::Bytes(wallet.to_vec()),
        ),
        (Value::Text("admin_key".into()), Value::Null),
    ]))?;
    let identity_id = certificate_id(IDENTITY, identity)?;
    verify(
        IDENTITY,
        identity,
        identity_signature,
        &identity_id,
        &trusted,
    )?;
    verify(
        MEMBERSHIP,
        membership,
        membership_signature,
        &certificate_id(MEMBERSHIP, membership)?,
        &trusted,
    )?;
    let public = identity_cert.fields.bytes::<32>("pubkey")?;
    let expiry = match (identity_cert.expiry, membership_cert.expiry) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let context = canonical::encode(Value::Map(vec![
        (Value::Text("chain_id".into()), identity_cert.chain.into()),
        (Value::Text("room".into()), Value::Text(room.into())),
        (Value::Text("gen".into()), 0u64.into()),
        (Value::Text("seat".into()), Value::Text(seat.clone())),
        (Value::Text("role".into()), Value::Text("owner".into())),
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
                ["manage", "read", "write"]
                    .into_iter()
                    .map(|s| Value::Text(s.into()))
                    .collect(),
            ),
        ),
        (
            Value::Text("expires_at".into()),
            expiry.map_or(Value::Null, |v| v.into()),
        ),
        (Value::Text("door_kind".into()), Value::Null),
        (Value::Text("bound_sender".into()), Value::Null),
        (Value::Text("forwarded_seat".into()), Value::Null),
    ]))?;
    canonical::encode(Value::Array(vec![
        Value::Bytes(identity_id),
        Value::Text(seat),
        Value::Bytes(public.to_vec()),
        Value::Bytes(context),
        Value::Bytes(wallet.to_vec()),
    ]))
}

/// Extract the encryption key from an already authenticated human/builder
/// identity. The caller must independently establish current room membership;
/// an identity ID match alone is not authorization.
pub fn recipient_key(identity: &[u8], expected_id: &[u8]) -> Result<Vec<u8>> {
    let cert = Certificate::decode(IDENTITY, identity)?;
    if certificate_id(IDENTITY, identity)? != expected_id {
        return Err(Error::CertificateId);
    }
    if !matches!(cert.fields.text("role")?.as_str(), "owner" | "builder") {
        return Err(Error::Authority);
    }
    Ok(cert.fields.bytes::<32>("enc_pubkey")?.to_vec())
}
