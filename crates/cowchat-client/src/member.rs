//! Native member sessions. An actor, @builder or connector member key proves
//! itself with the same session challenge a browser wallet signs, and opens a
//! hosted room key over the member relay: the server forwards only blinded
//! release bytes, and this client verifies shares and reconstructs the key.

use crate::{ClientError, CowchatClient};
use cowchat_core::{
    session_challenge_digest, session_possession_digest, FrameType, SessionChallengeRequest,
    SessionChallengeResponse, SessionProof, SESSION_MEMBER_AUDIENCE,
};
use k256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
use rand_core::OsRng;
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

/// Clock skew tolerated on the challenge expiry. The server stays authoritative
/// (60-second TTL, single use); this only avoids refusing a valid challenge on a
/// fast local clock.
const CHALLENGE_SKEW_MS: u64 = 5 * 60 * 1000;

fn keccak(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

fn auth(message: &str) -> ClientError {
    ClientError::Encryption(message.to_string())
}

/// Lowercase `0x` Ethereum address of a member key.
pub fn member_address(key: &SigningKey) -> String {
    let point = key.verifying_key().to_encoded_point(false);
    let hash = keccak(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

fn session_public_key(key: &SigningKey) -> String {
    format!(
        "0x{}",
        hex::encode(key.verifying_key().to_encoded_point(true).as_bytes())
    )
}

fn now_ms() -> Result<u64, ClientError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .ok_or_else(|| auth("system clock unavailable"))
}

/// The HTTPS (or loopback HTTP) challenge endpoint beside a `/ws` URL.
fn challenge_url(ws_url: &str) -> Result<reqwest::Url, ClientError> {
    let mut url = reqwest::Url::parse(ws_url).map_err(|_| auth("invalid Cowchat URL"))?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err(auth("Cowchat URL must be ws or wss")),
    };
    url.set_scheme(scheme)
        .map_err(|_| auth("invalid Cowchat URL"))?;
    url.set_path("/auth/session/challenge");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Check every echoed field, then sign the statement with the member key and
/// its digest with the session key.
pub(crate) fn prove(
    challenge: &SessionChallengeResponse,
    endpoint: &str,
    service_id: &[u8; 32],
    member: &SigningKey,
    session: &SigningKey,
    now_ms: u64,
) -> Result<SessionProof, ClientError> {
    if challenge.server_id != hex::encode(service_id)
        || challenge.endpoint != endpoint
        || challenge.audience != SESSION_MEMBER_AUDIENCE
        || !challenge.origin.is_empty()
        || challenge.principal_address != member_address(member)
        || challenge.session_public_key != session_public_key(session)
        || challenge.expires_at_ms.saturating_add(CHALLENGE_SKEW_MS) <= now_ms
    {
        return Err(auth(
            "session challenge does not match this member and server",
        ));
    }
    let digest = session_challenge_digest(challenge);
    let (signature, recovery): (Signature, _) = member
        .sign_prehash_recoverable(&digest)
        .map_err(|_| auth("cannot sign session challenge"))?;
    let mut principal = signature.to_bytes().to_vec();
    principal.push(recovery.to_byte());
    let possession: Signature = session
        .sign_prehash(&session_possession_digest(&digest))
        .map_err(|_| auth("cannot sign session possession"))?;
    Ok(SessionProof {
        nonce: challenge.nonce.clone(),
        principal_signature: format!("0x{}", hex::encode(principal)),
        possession_signature: format!("0x{}", hex::encode(possession.to_bytes())),
    })
}

impl CowchatClient {
    /// Register as `member:<address>` of the hosted service `service_id`,
    /// without the transport key. Aborts unless the server's challenge names
    /// exactly that service.
    pub async fn connect_member(
        ws_url: &str,
        service_id: &[u8; 32],
        member: &SigningKey,
        name: &str,
    ) -> Result<Self, ClientError> {
        let session = SigningKey::random(&mut OsRng);
        let endpoint = reqwest::Url::parse(ws_url)
            .map_err(|_| auth("invalid Cowchat URL"))?
            .to_string();
        let request = SessionChallengeRequest {
            principal_address: member_address(member),
            session_public_key: session_public_key(&session),
            audience: SESSION_MEMBER_AUDIENCE.to_string(),
            endpoint: endpoint.clone(),
        };
        let response = reqwest::Client::new()
            .post(challenge_url(ws_url)?)
            .json(&request)
            .send()
            .await
            .map_err(|e| ClientError::Ws(e.to_string()))?;
        if !response.status().is_success() {
            return Err(auth("session challenge refused"));
        }
        let challenge: SessionChallengeResponse = response
            .json()
            .await
            .map_err(|_| auth("invalid session challenge"))?;
        let proof = prove(
            &challenge,
            &endpoint,
            service_id,
            member,
            &session,
            now_ms()?,
        )?;
        Self::connect_ws_registered(ws_url, "", Some(proof), name, None, Vec::new()).await
    }

    /// Open the room's current key over the member relay and hold it for
    /// claims and replies. The owner-signed policy must belong to
    /// `expected_owner`. Returns the opened key epoch.
    pub async fn open_hosted_room_key(
        &mut self,
        room_id: &str,
        expected_owner: &[u8],
        member: &SigningKey,
    ) -> Result<u64, ClientError> {
        let context = self
            .request(
                FrameType::GetRoomKeyContext,
                serde_json::json!({"room_id": room_id, "member": member_address(member)}),
            )
            .await?
            .payload;
        let invalid = || auth("invalid room key context");
        let owner = context
            .pointer("/input/scope/owner")
            .and_then(|value| value.as_str())
            .ok_or_else(invalid)?;
        if owner.trim_start_matches("0x") != hex::encode(expected_owner) {
            return Err(auth("room belongs to another owner"));
        }
        let input = serde_json::to_string(context.get("input").ok_or_else(invalid)?)?;
        let custody = context
            .get("custody")
            .and_then(|value| value.as_str())
            .and_then(|value| hex::decode(value).ok())
            .ok_or_else(invalid)?;
        let key_epoch: u64 = context
            .get("key_epoch")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse().ok())
            .ok_or_else(invalid)?;
        let secret = Zeroizing::new(member.to_bytes());
        let attempt = cowboy_protocol_client_crypto::room_context::prepare_room_key(
            &input,
            secret.as_slice(),
            &mut OsRng,
        )
        .map_err(|e| auth(&e))?;
        attempt.validate_custody(&custody).map_err(|_| invalid())?;
        let relayed = self
            .request(
                FrameType::RelayRoomKeyOpen,
                serde_json::json!({"request": hex::encode(attempt.call_bytes())}),
            )
            .await?
            .payload;
        let responses = relayed
            .get("responses")
            .and_then(|value| value.as_array())
            .ok_or_else(invalid)?
            .iter()
            .map(|value| value.as_str().and_then(|hex| hex::decode(hex).ok()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        let key = attempt
            .open(&responses, &custody, now_ms()?)
            .map_err(|_| auth("room key did not open"))?;
        let key: [u8; 32] = key.as_slice().try_into().map_err(|_| invalid())?;
        self.set_hosted_room_key(room_id, key_epoch, key);
        Ok(key_epoch)
    }
}

/// What a hosted room holds for one addressed input.
#[derive(Debug)]
pub enum RoomTurn {
    /// The decrypted input; no reply with the deterministic id exists yet.
    Input(Box<cowchat_core::ChatMessage>),
    /// A reply with the deterministic id is already committed.
    AlreadyReplied,
    /// The input is not in the room's recent history.
    NotFound,
}

/// Messages a turn lookup scans (the hosted server's history bound).
const TURN_WINDOW: u32 = 1000;

/// The one reply id an actor member may commit for an input: `ar:` and the
/// first 32 hex of keccak256(member address bytes || input id). The hosted
/// writer deduplicates on it, so a repeated job cannot post a second reply, and
/// it stays inside the 64-byte locator bound when the reply mentions an actor.
pub fn actor_reply_id(member: &SigningKey, input_message_id: &str) -> String {
    let point = member.verifying_key().to_encoded_point(false);
    let mut preimage = keccak(&point.as_bytes()[1..])[12..].to_vec();
    preimage.extend_from_slice(input_message_id.as_bytes());
    format!("ar:{}", &hex::encode(keccak(&preimage))[..32])
}

impl CowchatClient {
    /// Find an addressed input and whether this member already replied to it.
    pub async fn find_room_turn(
        &self,
        room_id: &str,
        message_id: &str,
        reply_id: &str,
    ) -> Result<RoomTurn, ClientError> {
        let history = self.get_history(room_id, TURN_WINDOW, None).await?;
        if history.iter().any(|message| message.message_id == reply_id) {
            return Ok(RoomTurn::AlreadyReplied);
        }
        let Some(mut input) = history
            .into_iter()
            .find(|message| message.message_id == message_id)
        else {
            return Ok(RoomTurn::NotFound);
        };
        let epoch: u64 = input
            .key_epoch
            .as_deref()
            .and_then(|epoch| epoch.parse().ok())
            .ok_or_else(|| auth("room input is not hosted-encrypted"))?;
        let context = cowchat_core::room_crypto::Context {
            room_id,
            key_epoch: epoch,
            message_id,
        };
        input.content = cowchat_core::room_crypto::decrypt(
            self.hosted_key(room_id, epoch)?,
            &context,
            &input.content,
        )
        .map_err(|_| auth("room input did not decrypt"))?;
        Ok(RoomTurn::Input(Box::new(input)))
    }

    /// Encrypt under the room's newest held epoch and commit with the
    /// deterministic reply id.
    pub async fn send_hosted_reply(
        &self,
        input: &cowchat_core::ChatMessage,
        reply_id: &str,
        text: &str,
    ) -> Result<(), ClientError> {
        let epoch = self
            .newest_hosted_epoch(&input.room_id)
            .ok_or_else(|| auth("room key is not open"))?;
        let context = cowchat_core::room_crypto::Context {
            room_id: &input.room_id,
            key_epoch: epoch,
            message_id: reply_id,
        };
        let payload = Self::prepare_room_key_message(
            self.hosted_key(&input.room_id, epoch)?,
            &context,
            text,
            Some(&input.message_id),
            Vec::new(),
            serde_json::json!({}),
        )?;
        self.append_prepared_message(&payload).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000_000;

    fn challenge(member: &SigningKey, session: &SigningKey) -> SessionChallengeResponse {
        SessionChallengeResponse {
            server_id: hex::encode([7u8; 32]),
            endpoint: "wss://chat.example/ws".to_string(),
            audience: SESSION_MEMBER_AUDIENCE.to_string(),
            origin: String::new(),
            principal_address: member_address(member),
            session_public_key: session_public_key(session),
            nonce: "nonce".to_string(),
            expires_at_ms: NOW + 60_000,
        }
    }

    #[test]
    fn member_proof_recovers_to_the_member_and_rejects_foreign_challenges() {
        let member = SigningKey::from_slice(&[1u8; 32]).unwrap();
        let session = SigningKey::from_slice(&[2u8; 32]).unwrap();
        let good = challenge(&member, &session);
        let proof = prove(
            &good,
            "wss://chat.example/ws",
            &[7; 32],
            &member,
            &session,
            NOW,
        )
        .unwrap();

        let principal = hex::decode(proof.principal_signature.trim_start_matches("0x")).unwrap();
        let digest = session_challenge_digest(&good);
        let recovered = k256::ecdsa::VerifyingKey::recover_from_prehash(
            &digest,
            &Signature::from_slice(&principal[..64]).unwrap(),
            k256::ecdsa::RecoveryId::from_byte(principal[64]).unwrap(),
        )
        .unwrap();
        assert_eq!(&recovered, member.verifying_key());

        let mutations: [fn(&mut SessionChallengeResponse); 6] = [
            |c| c.server_id = hex::encode([8u8; 32]),
            |c| c.endpoint = "wss://other.example/ws".to_string(),
            |c| c.audience = "cowchat-browser".to_string(),
            |c| c.origin = "https://evil.example".to_string(),
            |c| c.principal_address = "0x0000000000000000000000000000000000000001".to_string(),
            |c| c.expires_at_ms = NOW - CHALLENGE_SKEW_MS,
        ];
        for mutate in mutations {
            let mut bad = good.clone();
            mutate(&mut bad);
            assert!(prove(
                &bad,
                "wss://chat.example/ws",
                &[7; 32],
                &member,
                &session,
                NOW
            )
            .is_err());
        }
    }

    #[test]
    fn actor_reply_id_is_short_deterministic_and_member_bound() {
        let member = SigningKey::from_slice(&[1u8; 32]).unwrap();
        let other = SigningKey::from_slice(&[3u8; 32]).unwrap();
        let input = "m".repeat(64);
        let id = actor_reply_id(&member, &input);
        assert_eq!(id.len(), 35);
        assert!(id.starts_with("ar:"));
        assert_eq!(id, actor_reply_id(&member, &input));
        assert_ne!(id, actor_reply_id(&other, &input));
        assert_ne!(id, actor_reply_id(&member, "another-input"));
    }

    #[test]
    fn challenge_endpoint_sits_beside_the_socket() {
        assert_eq!(
            challenge_url("wss://chat.example/ws?x=1").unwrap().as_str(),
            "https://chat.example/auth/session/challenge"
        );
        assert_eq!(
            challenge_url("ws://127.0.0.1:9229/ws").unwrap().as_str(),
            "http://127.0.0.1:9229/auth/session/challenge"
        );
        assert!(challenge_url("https://chat.example/ws").is_err());
    }
}
