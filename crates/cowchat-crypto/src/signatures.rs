use crate::{Error, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

pub(crate) fn verify_ed25519(key: &[u8], sig: &[u8], input: &[u8]) -> Result<()> {
    let key = VerifyingKey::from_bytes(key.try_into().map_err(|_| Error::Signature)?)
        .map_err(|_| Error::Signature)?;
    let sig = Signature::from_slice(sig).map_err(|_| Error::Signature)?;
    key.verify_strict(input, &sig).map_err(|_| Error::Signature)
}
pub(crate) fn sign_ed25519(seed: &[u8], input: &[u8]) -> Result<Vec<u8>> {
    let raw: Zeroizing<[u8; 32]> = Zeroizing::new(seed.try_into().map_err(|_| Error::Schema)?);
    Ok(SigningKey::from_bytes(&raw).sign(input).to_bytes().to_vec())
}
pub(crate) fn recover_wallet(sig: &[u8], input: &[u8]) -> Result<[u8; 20]> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    if sig.len() != 65 {
        return Err(Error::Signature);
    }
    let signature = Signature::from_slice(&sig[..64]).map_err(|_| Error::Signature)?;
    if signature.normalize_s().is_some() {
        return Err(Error::Signature);
    }
    let recid = RecoveryId::from_byte(sig[64]).ok_or(Error::Signature)?;
    let key = VerifyingKey::recover_from_prehash(&Keccak256::digest(input), &signature, recid)
        .map_err(|_| Error::Signature)?;
    let encoded = key.to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    digest[12..].try_into().map_err(|_| Error::Signature)
}
