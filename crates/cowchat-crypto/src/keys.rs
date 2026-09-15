//! The single room-key derivation implementation. Callers own and erase their
//! input/output buffers; intermediate key copies here are zeroized.
use crate::{canonical, Error, Result};
use ciborium::value::Value;
use hkdf::Hkdf;
use hpke::{
    aead::ChaCha20Poly1305, kdf::HkdfSha256, kem::X25519HkdfSha256 as Kem, Deserializable,
    Serializable,
};
use sha2::Sha256;
use zeroize::Zeroizing;

/// Preserves the original cow1 derivation for both legacy clients and v3.
pub fn derive_room_key(secret: &[u8], room_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    let mut info = b"cowchat-e2e-v1:".to_vec();
    info.extend_from_slice(room_id.as_bytes());
    let mut key = [0; 32];
    hk.expand(&info, &mut key)
        .expect("32 bytes is a valid HKDF output length");
    key
}
fn info(room: &str, generation: u64) -> Result<Vec<u8>> {
    if room.is_empty() || room.len() > 4096 {
        return Err(Error::Schema);
    }
    let mut info = b"cowchat/v3/roomkey".to_vec();
    info.extend_from_slice(&canonical::encode(Value::Array(vec![
        Value::Text(room.into()),
        Value::Integer(generation.into()),
    ]))?);
    Ok(info)
}

/// Returns the unwrapped 32-byte generation secret, not the derived AEAD key.
pub fn unwrap_room_key(
    room: &str,
    generation: u64,
    recipient_private: &[u8],
    enc: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    if ciphertext.len() != 48 {
        return Err(Error::Decrypt);
    }
    let raw: Zeroizing<[u8; 32]> =
        Zeroizing::new(recipient_private.try_into().map_err(|_| Error::Decrypt)?);
    let sk =
        <Kem as hpke::Kem>::PrivateKey::from_bytes(raw.as_slice()).map_err(|_| Error::Decrypt)?;
    let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(enc).map_err(|_| Error::Decrypt)?;
    let mut context = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, Kem>(
        &hpke::OpModeR::Base,
        &sk,
        &enc,
        &info(room, generation)?,
    )
    .map_err(|_| Error::Decrypt)?;
    let secret = context.open(ciphertext, b"").map_err(|_| Error::Decrypt)?;
    if secret.len() != 32 {
        return Err(Error::Decrypt);
    }
    Ok(secret)
}

/// Secure fresh encapsulation, returned as CBOR `[enc_bstr, ciphertext_bstr]`.
/// Tests use fixed published vectors only on the unwrap path; production does
/// not expose deterministic ephemeral-key or caller-nonce APIs.
pub fn wrap_room_key(
    room: &str,
    generation: u64,
    recipient_public: &[u8],
    generation_secret: &[u8],
) -> Result<Vec<u8>> {
    use rand::{rngs::StdRng, SeedableRng};
    let secret: Zeroizing<[u8; 32]> =
        Zeroizing::new(generation_secret.try_into().map_err(|_| Error::Schema)?);
    let pk =
        <Kem as hpke::Kem>::PublicKey::from_bytes(recipient_public).map_err(|_| Error::Schema)?;
    let mut rng = StdRng::try_from_os_rng().map_err(|_| Error::Random)?;
    let (enc, mut context) = hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, Kem, _>(
        &hpke::OpModeS::Base,
        &pk,
        &info(room, generation)?,
        &mut rng,
    )
    .map_err(|_| Error::Schema)?;
    let ct = context
        .seal(secret.as_slice(), b"")
        .map_err(|_| Error::Schema)?;
    canonical::encode(Value::Array(vec![
        Value::Bytes(enc.to_bytes().to_vec()),
        Value::Bytes(ct),
    ]))
}
