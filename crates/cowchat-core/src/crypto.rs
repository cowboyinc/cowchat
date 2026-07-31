//! Client-side end-to-end content encryption for Cowchat rooms.
//!
//! Message `content` is encrypted with ChaCha20-Poly1305 (IETF, 96-bit nonce)
//! under a per-room key derived from a pre-shared secret (`COWCHAT_ROOM_KEY`)
//! via HKDF-SHA256. The server never holds the secret — it only ever stores and
//! relays the opaque ciphertext blob. Encryption and decryption happen entirely
//! in the client.
//!
//! The ciphertext is self-describing: a `clw1:`-prefixed string carrying the
//! random nonce and the AEAD output, so it rides inside the existing `content`
//! field without any protocol/schema changes. The per-room key derivation binds
//! a blob to its room, so a ciphertext copied into another room won't decrypt.
//!
//! ChaCha20-Poly1305 (IETF) is chosen over XChaCha20 for cross-language reach:
//! it's available in both RustCrypto and Python's `cryptography` package, so the
//! Rust and Python clients interoperate byte-for-byte. A fresh random 96-bit
//! nonce is generated per message; per-room keys keep per-key message counts far
//! below the birthday bound where random-nonce reuse becomes a concern.

use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

/// Prefix marking a string as a Cowchat v1 encrypted blob.
/// Protocol identifier — frozen for compatibility with pre-rename data/clients. Do not rename.
const PREFIX: &str = "clw1:";
/// ChaCha20-Poly1305 (IETF) nonce length.
const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("not an encrypted blob")]
    NotCiphertext,
    #[error("malformed ciphertext")]
    Malformed,
    #[error("decryption failed (wrong key or tampered ciphertext)")]
    DecryptFailed,
}

/// True if `s` looks like a Cowchat encrypted blob.
pub fn is_ciphertext(s: &str) -> bool {
    s.starts_with(PREFIX)
}

/// Generate a fresh random pre-shared room secret (32 bytes, hex-encoded),
/// suitable for use as `COWCHAT_ROOM_KEY`. Every agent in a group should be
/// given the same value to read each other's end-to-end-encrypted messages.
pub fn generate_secret() -> String {
    use chacha20poly1305::aead::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Derive the 32-byte AEAD key for `room_id` from the pre-shared `secret`.
///
/// Binding the room id into the HKDF `info` means the same passphrase yields a
/// distinct key per room, so a leaked per-room key doesn't compromise other
/// rooms and a blob can't be replayed across rooms.
pub fn derive_room_key(secret: &[u8], room_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    // Protocol identifier — frozen for compatibility with pre-rename data/clients. Do not rename.
    let mut info = b"clawchat-e2e-v1:".to_vec();
    info.extend_from_slice(room_id.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(&info, &mut key)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Encrypt `plaintext` for `room_id`, returning a self-describing `clw1:` blob.
pub fn encrypt(secret: &[u8], room_id: &str, plaintext: &str) -> String {
    let key = derive_room_key(secret, room_id);
    let cipher =
        ChaCha20Poly1305::new_from_slice(&key).expect("32-byte key is valid for ChaCha20-Poly1305");
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("ChaCha20-Poly1305 encryption does not fail for valid inputs");
    let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
    blob.extend_from_slice(nonce.as_slice());
    blob.extend_from_slice(&ct);
    format!("{}{}", PREFIX, B64.encode(blob))
}

/// Decrypt a `clw1:` blob produced by [`encrypt`] for the same `room_id` and
/// `secret`. Returns [`CryptoError::NotCiphertext`] if `blob` isn't a Cowchat
/// blob, and [`CryptoError::DecryptFailed`] on a wrong key or tampered input.
pub fn decrypt(secret: &[u8], room_id: &str, blob: &str) -> Result<String, CryptoError> {
    let b64 = blob
        .strip_prefix(PREFIX)
        .ok_or(CryptoError::NotCiphertext)?;
    let raw = B64.decode(b64).map_err(|_| CryptoError::Malformed)?;
    if raw.len() < NONCE_LEN {
        return Err(CryptoError::Malformed);
    }
    let (nonce_bytes, ct) = raw.split_at(NONCE_LEN);
    let key = derive_room_key(secret, room_id);
    let cipher =
        ChaCha20Poly1305::new_from_slice(&key).expect("32-byte key is valid for ChaCha20-Poly1305");
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| CryptoError::DecryptFailed)?;
    String::from_utf8(pt).map_err(|_| CryptoError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"correct horse battery staple";
    const ROOM: &str = "room-abc";

    #[test]
    fn round_trip() {
        let blob = encrypt(SECRET, ROOM, "hello world");
        assert!(is_ciphertext(&blob));
        assert_eq!(decrypt(SECRET, ROOM, &blob).unwrap(), "hello world");
    }

    #[test]
    fn decrypts_v031_ciphertext() {
        // Blob generated by clawchat-core at tag v0.3.1 (pre-rename). If this
        // stops decrypting, the rename broke wire compatibility.
        const V031_BLOB: &str =
            "clw1:wD4E7uZ9VECvN7VXyiSKTADOMJyTT9QZTuGeXfRMyjFngay5WkfPer669ccZkZphiHqe+dE";
        assert_eq!(
            decrypt(b"fixture-secret", "fixture-room", V031_BLOB).unwrap(),
            "cowchat fixture plaintext"
        );
    }

    #[test]
    fn distinct_nonces_per_call() {
        // Same plaintext encrypts to different blobs (random nonce).
        let a = encrypt(SECRET, ROOM, "same");
        let b = encrypt(SECRET, ROOM, "same");
        assert_ne!(a, b);
        assert_eq!(decrypt(SECRET, ROOM, &a).unwrap(), "same");
        assert_eq!(decrypt(SECRET, ROOM, &b).unwrap(), "same");
    }

    #[test]
    fn wrong_secret_fails() {
        let blob = encrypt(SECRET, ROOM, "secret message");
        let err = decrypt(b"a different secret", ROOM, &blob).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptFailed));
    }

    #[test]
    fn wrong_room_fails() {
        // Room id is bound into the key, so a different room can't decrypt.
        let blob = encrypt(SECRET, ROOM, "secret message");
        let err = decrypt(SECRET, "other-room", &blob).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptFailed));
    }

    #[test]
    fn tamper_fails() {
        let blob = encrypt(SECRET, ROOM, "secret message");
        // Flip the last base64 char to corrupt the ciphertext/tag.
        let mut bytes = blob.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0b0000_0001;
        // Keep it a valid char from the alphabet.
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(decrypt(SECRET, ROOM, &tampered).is_err());
    }

    #[test]
    fn plaintext_is_not_ciphertext() {
        assert!(!is_ciphertext("just a normal message"));
        let err = decrypt(SECRET, ROOM, "just a normal message").unwrap_err();
        assert!(matches!(err, CryptoError::NotCiphertext));
    }

    #[test]
    fn derived_keys_differ_per_room() {
        assert_ne!(
            derive_room_key(SECRET, "room-a"),
            derive_room_key(SECRET, "room-b")
        );
    }
}
