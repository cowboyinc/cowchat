use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use cowchat_core::{
    session_challenge_digest, session_possession_digest, SessionChallengeRequest,
    SessionChallengeResponse, SessionProof, SESSION_BROWSER_AUDIENCE, SESSION_MEMBER_AUDIENCE,
};
use dashmap::DashMap;
use k256::ecdsa::{signature::hazmat::PrehashVerifier, RecoveryId, Signature, VerifyingKey};
use rand::{rngs::OsRng, RngCore};
use sha3::{Digest as _, Keccak256};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const CHALLENGE_TTL_MS: u64 = 60_000;
const MAX_PENDING_CHALLENGES: usize = 4_096;
const MAX_PENDING_PER_PRINCIPAL: usize = 4;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPrincipal {
    pub principal_address: String,
    pub agent_id: String,
}

#[derive(Clone)]
struct PendingChallenge {
    response: SessionChallengeResponse,
    digest: [u8; 32],
}

#[derive(Clone)]
pub struct SessionAuth {
    server_id: String,
    endpoint: String,
    pending: Arc<DashMap<String, PendingChallenge>>,
}

impl SessionAuth {
    pub fn new(service_id: [u8; 32], endpoint: String) -> Result<Self, &'static str> {
        let endpoint = normalize_endpoint(&endpoint)?;
        Ok(Self {
            server_id: hex::encode(service_id),
            endpoint,
            pending: Arc::new(DashMap::new()),
        })
    }

    pub fn issue(
        &self,
        request: SessionChallengeRequest,
        origin: &str,
    ) -> Result<SessionChallengeResponse, &'static str> {
        let now = now_ms()?;
        self.pending
            .retain(|_, challenge| challenge.response.expires_at_ms > now);
        if self.pending.len() >= MAX_PENDING_CHALLENGES {
            return Err("too many pending sessions");
        }

        let principal_address = normalize_address(&request.principal_address)?;
        if self
            .pending
            .iter()
            .filter(|challenge| challenge.response.principal_address == principal_address)
            .count()
            >= MAX_PENDING_PER_PRINCIPAL
        {
            return Err("too many pending sessions for principal");
        }
        let session_public_key = normalize_public_key(&request.session_public_key)?;
        if normalize_endpoint(&request.endpoint)? != self.endpoint {
            return Err("session endpoint does not match this server");
        }
        let audience = normalize_audience(&request.audience, origin)?;

        let mut nonce_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
        let response = SessionChallengeResponse {
            server_id: self.server_id.clone(),
            endpoint: self.endpoint.clone(),
            audience,
            origin: origin.to_owned(),
            principal_address,
            session_public_key,
            nonce: nonce.clone(),
            expires_at_ms: now + CHALLENGE_TTL_MS,
        };
        let digest = session_challenge_digest(&response);
        self.pending.insert(
            nonce,
            PendingChallenge {
                response: response.clone(),
                digest,
            },
        );
        Ok(response)
    }

    /// Consume and verify a session challenge. Removal happens before any
    /// signature work, so malformed attempts cannot be replayed as an oracle.
    pub fn verify(
        &self,
        proof: &SessionProof,
        origin: &str,
    ) -> Result<MemberPrincipal, &'static str> {
        let (_, pending) = self
            .pending
            .remove(&proof.nonce)
            .ok_or("unknown or already used session challenge")?;
        if pending.response.expires_at_ms <= now_ms()? {
            return Err("expired session challenge");
        }
        if pending.response.origin != origin {
            return Err("session client class does not match challenge");
        }

        let principal_signature = decode_hex::<65>(&proof.principal_signature)?;
        let signature = Signature::from_slice(&principal_signature[..64])
            .map_err(|_| "invalid principal signature")?;
        if signature.normalize_s().is_some() {
            return Err("non-canonical principal signature");
        }
        let recovery_byte = match principal_signature[64] {
            27 | 28 => principal_signature[64] - 27,
            value => value,
        };
        let recovery =
            RecoveryId::from_byte(recovery_byte).ok_or("invalid principal recovery id")?;
        let principal_key =
            VerifyingKey::recover_from_prehash(&pending.digest, &signature, recovery)
                .map_err(|_| "invalid principal signature")?;
        if address_for_key(&principal_key) != pending.response.principal_address {
            return Err("principal signature does not match challenge");
        }

        let session_bytes = decode_prefixed_hex::<33>(&pending.response.session_public_key)?;
        let session_key = VerifyingKey::from_sec1_bytes(&session_bytes)
            .map_err(|_| "invalid session public key")?;
        let possession = decode_hex::<64>(&proof.possession_signature)?;
        let possession =
            Signature::from_slice(&possession).map_err(|_| "invalid possession signature")?;
        if possession.normalize_s().is_some() {
            return Err("non-canonical possession signature");
        }
        session_key
            .verify_prehash(&session_possession_digest(&pending.digest), &possession)
            .map_err(|_| "invalid possession signature")?;

        Ok(MemberPrincipal {
            agent_id: format!("member:{}", pending.response.principal_address),
            principal_address: pending.response.principal_address,
        })
    }
}

fn normalize_endpoint(value: &str) -> Result<String, &'static str> {
    let url = url::Url::parse(value).map_err(|_| "invalid session endpoint")?;
    if !matches!(url.scheme(), "ws" | "wss")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/ws"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid session endpoint");
    }
    Ok(url.to_string())
}

fn now_ms() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|_| "system clock is before the Unix epoch")
}

fn normalize_address(value: &str) -> Result<String, &'static str> {
    let lower = value.to_ascii_lowercase();
    let bytes = decode_prefixed_hex::<20>(&lower)?;
    if bytes == [0; 20] {
        return Err("zero principal address");
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn normalize_public_key(value: &str) -> Result<String, &'static str> {
    let bytes = decode_prefixed_hex::<33>(value)?;
    VerifyingKey::from_sec1_bytes(&bytes).map_err(|_| "invalid session public key")?;
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn normalize_audience(value: &str, origin: &str) -> Result<String, &'static str> {
    let expected = if origin.is_empty() {
        SESSION_MEMBER_AUDIENCE
    } else {
        SESSION_BROWSER_AUDIENCE
    };
    if value != expected {
        return Err("session audience does not match client class");
    }
    Ok(value.to_owned())
}

pub(crate) fn address_for_key(key: &VerifyingKey) -> String {
    let encoded = key.to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], &'static str> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(raw).map_err(|_| "invalid hex")?;
    bytes.try_into().map_err(|_| "invalid hex length")
}

fn decode_prefixed_hex<const N: usize>(value: &str) -> Result<[u8; N], &'static str> {
    if !value.starts_with("0x") {
        return Err("hex value must be 0x-prefixed");
    }
    decode_hex(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};

    fn proof(auth: &SessionAuth) -> (SessionProof, String) {
        let wallet = SigningKey::random(&mut OsRng);
        let ephemeral = SigningKey::random(&mut OsRng);
        let principal_address = address_for_key(wallet.verifying_key());
        let challenge = auth
            .issue(
                SessionChallengeRequest {
                    principal_address: principal_address.clone(),
                    session_public_key: format!(
                        "0x{}",
                        hex::encode(ephemeral.verifying_key().to_encoded_point(true).as_bytes())
                    ),
                    audience: SESSION_BROWSER_AUDIENCE.into(),
                    endpoint: "wss://chat.example/ws".into(),
                },
                "https://dashboard.example",
            )
            .unwrap();
        let digest = session_challenge_digest(&challenge);
        let (signature, recovery) = wallet.sign_prehash_recoverable(&digest).unwrap();
        let mut principal_signature = signature.to_bytes().to_vec();
        principal_signature.push(recovery.to_byte());
        let possession: Signature = ephemeral
            .sign_prehash(&session_possession_digest(&digest))
            .unwrap();
        (
            SessionProof {
                nonce: challenge.nonce,
                principal_signature: format!("0x{}", hex::encode(principal_signature)),
                possession_signature: format!("0x{}", hex::encode(possession.to_bytes())),
            },
            principal_address,
        )
    }

    #[test]
    fn challenge_is_single_use_and_derives_member_agent_id() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (proof, address) = proof(&auth);
        let principal = auth.verify(&proof, "https://dashboard.example").unwrap();
        assert_eq!(principal.principal_address, address);
        assert_eq!(principal.agent_id, format!("member:{address}"));
        assert!(auth.verify(&proof, "https://dashboard.example").is_err());
    }

    #[test]
    fn possession_signature_is_mandatory() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (valid, _) = proof(&auth);
        let mut invalid = valid.clone();
        invalid.possession_signature = format!("0x{}", "00".repeat(64));
        assert!(auth.verify(&invalid, "https://dashboard.example").is_err());
        assert!(auth.verify(&valid, "https://dashboard.example").is_err());
    }

    #[test]
    fn possession_signature_from_another_valid_session_is_rejected() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (mut candidate, _) = proof(&auth);
        let (other, _) = proof(&auth);
        candidate.possession_signature = other.possession_signature;
        assert!(auth
            .verify(&candidate, "https://dashboard.example")
            .is_err());
    }

    #[test]
    fn pending_challenges_are_bounded_per_principal() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let principal = SigningKey::random(&mut OsRng);
        let request = |key: &SigningKey| SessionChallengeRequest {
            principal_address: address_for_key(principal.verifying_key()),
            session_public_key: format!(
                "0x{}",
                hex::encode(key.verifying_key().to_encoded_point(true).as_bytes())
            ),
            audience: SESSION_BROWSER_AUDIENCE.into(),
            endpoint: "wss://chat.example/ws".into(),
        };
        for _ in 0..MAX_PENDING_PER_PRINCIPAL {
            let session = SigningKey::random(&mut OsRng);
            auth.issue(request(&session), "https://dashboard.example")
                .unwrap();
        }
        let session = SigningKey::random(&mut OsRng);
        assert!(auth
            .issue(request(&session), "https://dashboard.example")
            .is_err());
    }

    #[test]
    fn audience_must_match_client_class() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let wallet = SigningKey::random(&mut OsRng);
        assert!(auth
            .issue(
                SessionChallengeRequest {
                    principal_address: address_for_key(wallet.verifying_key()),
                    session_public_key: format!(
                        "0x{}",
                        hex::encode(wallet.verifying_key().to_encoded_point(true).as_bytes())
                    ),
                    audience: SESSION_MEMBER_AUDIENCE.into(),
                    endpoint: "wss://chat.example/ws".into(),
                },
                "https://dashboard.example",
            )
            .is_err());
    }

    #[test]
    fn native_challenge_has_an_empty_origin() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let principal = SigningKey::random(&mut OsRng);
        let session = SigningKey::random(&mut OsRng);
        let challenge = auth
            .issue(
                SessionChallengeRequest {
                    principal_address: address_for_key(principal.verifying_key()),
                    session_public_key: format!(
                        "0x{}",
                        hex::encode(session.verifying_key().to_encoded_point(true).as_bytes())
                    ),
                    audience: SESSION_MEMBER_AUDIENCE.into(),
                    endpoint: "wss://chat.example/ws".into(),
                },
                "",
            )
            .unwrap();
        assert!(challenge.origin.is_empty());
    }

    #[test]
    fn challenge_rejects_another_endpoint_and_reverse_audience() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let key = SigningKey::random(&mut OsRng);
        let request = |audience: &str, endpoint: &str| SessionChallengeRequest {
            principal_address: address_for_key(key.verifying_key()),
            session_public_key: format!(
                "0x{}",
                hex::encode(key.verifying_key().to_encoded_point(true).as_bytes())
            ),
            audience: audience.into(),
            endpoint: endpoint.into(),
        };
        assert!(auth
            .issue(
                request(SESSION_MEMBER_AUDIENCE, "wss://other.example/ws"),
                ""
            )
            .is_err());
        assert!(auth
            .issue(
                request(SESSION_BROWSER_AUDIENCE, "wss://chat.example/ws"),
                ""
            )
            .is_err());
    }

    #[test]
    fn proof_is_bound_to_origin_and_expiry() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (origin_bound, _) = proof(&auth);
        assert!(auth.verify(&origin_bound, "https://other.example").is_err());

        let (expired, _) = proof(&auth);
        auth.pending
            .get_mut(&expired.nonce)
            .unwrap()
            .response
            .expires_at_ms = 0;
        assert!(auth.verify(&expired, "https://dashboard.example").is_err());
    }

    #[test]
    fn ethereum_recovery_ids_are_accepted() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (mut proof, _) = proof(&auth);
        let mut bytes = decode_hex::<65>(&proof.principal_signature).unwrap();
        bytes[64] += 27;
        proof.principal_signature = format!("0x{}", hex::encode(bytes));
        assert!(auth.verify(&proof, "https://dashboard.example").is_ok());
    }

    #[test]
    fn high_s_signatures_are_rejected() {
        let auth = SessionAuth::new([7; 32], "wss://chat.example/ws".into()).unwrap();
        let (mut proof, _) = proof(&auth);
        let mut principal = decode_hex::<65>(&proof.principal_signature).unwrap();
        let signature = Signature::from_slice(&principal[..64]).unwrap();
        let high_s =
            Signature::from_scalars(signature.r().to_bytes(), (-signature.s()).to_bytes()).unwrap();
        principal[..64].copy_from_slice(&high_s.to_bytes());
        proof.principal_signature = format!("0x{}", hex::encode(principal));

        assert!(matches!(
            auth.verify(&proof, "https://dashboard.example"),
            Err("non-canonical principal signature")
        ));
    }
}
