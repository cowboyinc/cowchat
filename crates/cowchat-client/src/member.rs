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

/// The owner plus `members`, lowercased, sorted and deduplicated: the only
/// roster form room policies accept.
fn canonical_roster(owner: &str, members: &[String]) -> Result<Vec<String>, ClientError> {
    let mut roster = vec![owner.to_string()];
    for member in members {
        let member = member.to_ascii_lowercase();
        let hex = member.strip_prefix("0x").unwrap_or(&member);
        if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(auth("member must be a 0x-prefixed 20-byte address"));
        }
        roster.push(format!("0x{hex}"));
    }
    roster.sort_unstable();
    roster.dedup();
    Ok(roster)
}

/// A hosted Cowchat service as registered on chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedService {
    pub service_id: [u8; 32],
    pub operator: String,
    pub endpoint: String,
}

/// Find a hosted Cowchat service in the chain's registry
/// (`GET <rpc_url>/cowchat/services`). With `operator`, the service that
/// operator runs; otherwise the chain must list exactly one.
pub async fn discover_hosted_service(
    rpc_url: &str,
    operator: Option<&str>,
) -> Result<HostedService, ClientError> {
    let url = format!("{}/cowchat/services", rpc_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| ClientError::Ws(e.to_string()))?;
    if !response.status().is_success() {
        return Err(auth("chain service registry unavailable"));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| auth("invalid chain service registry"))?;
    let invalid = || auth("invalid chain service registry");
    let mut services = Vec::new();
    for entry in body
        .get("services")
        .and_then(|v| v.as_array())
        .ok_or_else(invalid)?
    {
        let field = |name: &str| entry.get(name).and_then(|v| v.as_str()).ok_or_else(invalid);
        let mut service_id = [0u8; 32];
        hex::decode_to_slice(
            field("service_id")?.trim_start_matches("0x"),
            &mut service_id,
        )
        .map_err(|_| invalid())?;
        services.push(HostedService {
            service_id,
            operator: field("operator")?.to_ascii_lowercase(),
            endpoint: field("endpoint")?.to_string(),
        });
    }
    match operator {
        Some(operator) => {
            let operator = operator.to_ascii_lowercase();
            services
                .into_iter()
                .find(|s| s.operator == operator)
                .ok_or_else(|| auth("no Cowchat service registered by that operator"))
        }
        None if services.len() == 1 => Ok(services.remove(0)),
        None if services.is_empty() => Err(auth("no Cowchat service is registered on this chain")),
        None => Err(auth(
            "several Cowchat services are registered; choose one by operator",
        )),
    }
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
        expected_owner: &[u8; 20],
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

    /// Create an encrypted hosted room owned by `owner`, the member this
    /// session proved. Every pinned holder attests the owner-signed setup
    /// before this client wraps a fresh room key; the key never leaves this
    /// process and is held as epoch 0 for this session.
    pub async fn create_hosted_room(
        &mut self,
        name: &str,
        owner: &SigningKey,
        service_id: &[u8; 32],
        members: &[String],
    ) -> Result<cowchat_core::Room, ClientError> {
        use cowboy_protocol_client_crypto::room_setup::SetupAttempt;
        use rand_core::RngCore;

        let invalid = || auth("invalid room key setup context");
        let room_id = uuid::Uuid::new_v4().to_string();
        let owner_address = member_address(owner);
        let roster = canonical_roster(&owner_address, members)?;
        let mut prepare =
            serde_json::json!({"room_id": room_id, "name": name, "owner": owner_address});
        if roster.len() > 1 {
            prepare["members"] = serde_json::json!(roster);
        }
        let context = self
            .request(FrameType::PrepareRoomKey, prepare)
            .await?
            .payload;
        let setup = context
            .get("setup")
            .filter(|v| v.is_object())
            .ok_or_else(invalid)?;
        let scope = context
            .get("scope")
            .filter(|v| v.is_object())
            .ok_or_else(invalid)?;
        let secret = Zeroizing::new(owner.to_bytes());
        let attempt = SetupAttempt::prepare(&setup.to_string(), secret.as_slice(), now_ms()?)
            .map_err(|e| auth(&e))?;
        // Release the owner-signed request only for exactly the room asked for.
        let intent: serde_json::Value = serde_json::from_str(&attempt.intent_projection())?;
        if intent["service_id"] != format!("0x{}", hex::encode(service_id))
            || intent["owner"] != owner_address
            || intent["room_id"] != room_id
            || intent["policy_epoch"] != "0"
            || intent["key_epoch"] != "0"
            || intent["members"] != serde_json::json!(roster)
        {
            return Err(auth("room key setup does not match the requested room"));
        }
        let attested = self
            .request(
                FrameType::AttestRoomKeySetup,
                serde_json::json!({"request": hex::encode(attempt.request_bytes())}),
            )
            .await?
            .payload;
        let responses = attested
            .get("responses")
            .filter(|v| {
                v.as_array()
                    .is_some_and(|a| a.iter().all(|r| r.is_string()))
            })
            .ok_or_else(|| auth("invalid room key setup response"))?;
        let mut key = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(key.as_mut_slice());
        let preparation = attempt
            .finalize_initial(
                &responses.to_string(),
                &scope.to_string(),
                key.as_slice(),
                secret.as_slice(),
                now_ms()?,
                &mut OsRng,
            )
            .map_err(|e| auth(&e))?;
        let preparation: serde_json::Value = serde_json::from_str(&preparation)?;
        // The server publishes, waits for courier finality (up to 120 s) and
        // fences every holder before it answers.
        let activated = self
            .request_within(
                FrameType::ActivateRoomKey,
                serde_json::json!({"room_id": room_id, "name": name, "preparation": preparation}),
                std::time::Duration::from_secs(180),
            )
            .await?
            .payload;
        let room: cowchat_core::Room = serde_json::from_value(activated)
            .map_err(|_| auth("invalid encrypted-room activation"))?;
        if room.room_id != room_id || !room.encrypted {
            return Err(auth("invalid encrypted-room activation"));
        }
        self.set_hosted_room_key(&room_id, 0, *key);
        Ok(room)
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

    /// Serve `body` once per connection as a JSON HTTP response.
    async fn registry(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn discovery_picks_the_only_service_or_the_named_operator() {
        let one = registry(concat!(
            r#"{"services":[{"service_id":"0x"#,
            "0707070707070707070707070707070707070707070707070707070707070707",
            r#"","operator":"0xAA00000000000000000000000000000000000001","endpoint":"wss://a.example/ws","registered_at_block":3}]}"#
        ))
        .await;
        let found = discover_hosted_service(&one, None).await.unwrap();
        assert_eq!(found.service_id, [7; 32]);
        assert_eq!(found.endpoint, "wss://a.example/ws");
        assert_eq!(
            discover_hosted_service(&one, Some("0xaa00000000000000000000000000000000000001"))
                .await
                .unwrap(),
            found
        );
        assert!(discover_hosted_service(&one, Some("0x01")).await.is_err());

        let none = registry(r#"{"services":[]}"#).await;
        assert!(discover_hosted_service(&none, None).await.is_err());

        let two = registry(concat!(
            r#"{"services":[{"service_id":"0x"#,
            "0707070707070707070707070707070707070707070707070707070707070707",
            r#"","operator":"0x01","endpoint":"wss://a.example/ws"},{"service_id":"0x"#,
            "0808080808080808080808080808080808080808080808080808080808080808",
            r#"","operator":"0x02","endpoint":"wss://b.example/ws"}]}"#
        ))
        .await;
        assert!(discover_hosted_service(&two, None).await.is_err());
        let b = discover_hosted_service(&two, Some("0x02")).await.unwrap();
        assert_eq!(
            (b.service_id, b.endpoint.as_str()),
            ([8; 32], "wss://b.example/ws")
        );
    }

    #[test]
    fn roster_is_canonical_and_always_holds_the_owner() {
        let owner = "0x0000000000000000000000000000000000000002";
        let roster = canonical_roster(
            owner,
            &[
                "0xAA00000000000000000000000000000000000003".to_string(),
                "0x0000000000000000000000000000000000000001".to_string(),
                owner.to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            roster,
            [
                "0x0000000000000000000000000000000000000001",
                owner,
                "0xaa00000000000000000000000000000000000003",
            ]
        );
        assert_eq!(canonical_roster(owner, &[]).unwrap(), [owner]);
        assert!(canonical_roster(owner, &["0x01".to_string()]).is_err());
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
