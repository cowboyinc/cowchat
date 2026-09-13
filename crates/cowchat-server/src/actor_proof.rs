//! Finalized actor-control reads for enrollment and background revocation. This module has no write RPC,
//! wake path, or constructor accepting a caller's trust anchor.
use cowboy_protocol_codec::{
    decode_trusted_checkpoint_v1, MAX_FINALIZED_STATE_PROOF_BUNDLE_BYTES_V1,
};
use cowboy_protocol_light_client::verify_finalized_state_proof_v1;
use cowchat_crypto::canonical;
use sha2::{Digest, Sha256};
use std::time::Duration;

mod release {
    include!(concat!(env!("OUT_DIR"), "/actor_checkpoint.rs"));
}
pub const ACTOR_CONTROL_KEY: &[u8] = b"__cowchat/control/v1";
const MAX_AGE_MS: u64 = 60_000;
const MAX_FUTURE_SKEW_MS: u64 = 5_000;

#[derive(Debug, thiserror::Error)]
pub enum ActorProofError {
    #[error("actor enrollment requires a valid release-pinned checkpoint")]
    Checkpoint,
    #[error("invalid configured actor proof RPC origin")]
    Origin,
    #[error("actor proof fetch failed")]
    Fetch,
    #[error("actor proof is invalid, absent, or stale")]
    Proof,
}

/// The only production constructor embeds its anchor at build time. Endpoint
/// configuration chooses a courier, never the chain or consensus identity.
#[derive(Clone)]
pub struct ActorProofAuthority {
    checkpoint: Vec<u8>,
    checkpoint_height: u64,
    endpoint: reqwest::Url,
    client: reqwest::Client,
}

/// Private fields prevent an HTTP caller from manufacturing a verified record.
/// The room store must still compare its height/root against the local rollback
/// floor and recheck its room generations in the enrollment transaction.
pub struct VerifiedActorControl {
    actor: [u8; 20],
    chain_id: u64,
    chain_instance: [u8; 32],
    height: u64,
    block_hash: [u8; 32],
    state_root: [u8; 32],
    timestamp: u64,
    controller: [u8; 20],
    commitment: [u8; 32],
    authorization_generation: u64,
}
/// An authenticated absence is distinct from a missing/invalid RPC response.
pub enum VerifiedActorState {
    Present(VerifiedActorControl),
    Absent(VerifiedActorAbsence),
}

pub struct VerifiedActorAbsence {
    actor: [u8; 20],
    chain_id: u64,
    chain_instance: [u8; 32],
    height: u64,
    block_hash: [u8; 32],
    state_root: [u8; 32],
    timestamp: u64,
}
impl VerifiedActorAbsence {
    pub fn actor(&self) -> [u8; 20] {
        self.actor
    }
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
    pub fn chain_instance(&self) -> [u8; 32] {
        self.chain_instance
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash
    }
    pub fn state_root(&self) -> [u8; 32] {
        self.state_root
    }
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
    pub fn is_fresh_at(&self, now: u64) -> bool {
        fresh(now, self.timestamp)
    }
}
impl VerifiedActorControl {
    pub fn actor(&self) -> [u8; 20] {
        self.actor
    }
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
    pub fn chain_instance(&self) -> [u8; 32] {
        self.chain_instance
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash
    }
    pub fn state_root(&self) -> [u8; 32] {
        self.state_root
    }
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
    pub fn controller(&self) -> [u8; 20] {
        self.controller
    }
    pub fn commitment(&self) -> [u8; 32] {
        self.commitment
    }
    pub fn authorization_generation(&self) -> u64 {
        self.authorization_generation
    }
    pub fn is_fresh_at(&self, now: u64) -> bool {
        fresh(now, self.timestamp)
    }
}
fn fresh(now: u64, timestamp: u64) -> bool {
    timestamp <= now.saturating_add(MAX_FUTURE_SKEW_MS)
        && now.saturating_sub(timestamp) <= MAX_AGE_MS
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl ActorProofAuthority {
    pub fn from_release(rpc_origin: &str) -> Result<Self, ActorProofError> {
        if release::CHECKPOINT.is_empty()
            || Sha256::digest(release::CHECKPOINT).as_slice() != release::DIGEST
        {
            return Err(ActorProofError::Checkpoint);
        }
        let checkpoint = decode_trusted_checkpoint_v1(release::CHECKPOINT)
            .map_err(|_| ActorProofError::Checkpoint)?;
        let mut endpoint = reqwest::Url::parse(rpc_origin).map_err(|_| ActorProofError::Origin)?;
        let local = endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && local))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ActorProofError::Origin);
        }
        endpoint.set_path(&format!(
            "{}/proof/finalized-state",
            endpoint.path().trim_end_matches('/')
        ));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| ActorProofError::Fetch)?;
        Ok(Self {
            checkpoint: release::CHECKPOINT.to_vec(),
            checkpoint_height: checkpoint.height,
            endpoint,
            client,
        })
    }

    /// Fetch highest available finalized state from the configured service RPC.
    /// No proof bytes, URL, trust anchor, or time value come from the HTTP client.
    pub async fn fetch(&self, actor: [u8; 20]) -> Result<VerifiedActorControl, ActorProofError> {
        match self.fetch_state(actor).await? {
            VerifiedActorState::Present(control) => Ok(control),
            VerifiedActorState::Absent(_) => Err(ActorProofError::Proof),
        }
    }

    pub async fn fetch_state(
        &self,
        actor: [u8; 20],
    ) -> Result<VerifiedActorState, ActorProofError> {
        let mut response=self.client.post(self.endpoint.clone()).json(&serde_json::json!({
            "checkpoint_height":self.checkpoint_height,
            "claims":[{"actor":format!("0x{}",hex(&actor)),"logical_key_hex":format!("0x{}",hex(ACTOR_CONTROL_KEY))}]
        })).send().await.map_err(|_|ActorProofError::Fetch)?;
        if !response.status().is_success() {
            return Err(ActorProofError::Fetch);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ActorProofError::Fetch)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_FINALIZED_STATE_PROOF_BUNDLE_BYTES_V1 {
                return Err(ActorProofError::Proof);
            }
            bytes.extend_from_slice(&chunk);
        }
        let now = u64::try_from(chrono::Utc::now().timestamp_millis())
            .map_err(|_| ActorProofError::Proof)?;
        self.verify_state(actor, &bytes, now)
    }

    #[cfg(test)]
    fn verify(
        &self,
        actor: [u8; 20],
        bytes: &[u8],
        now: u64,
    ) -> Result<VerifiedActorControl, ActorProofError> {
        match self.verify_state(actor, bytes, now)? {
            VerifiedActorState::Present(control) => Ok(control),
            VerifiedActorState::Absent(_) => Err(ActorProofError::Proof),
        }
    }

    fn verify_state(
        &self,
        actor: [u8; 20],
        bytes: &[u8],
        now: u64,
    ) -> Result<VerifiedActorState, ActorProofError> {
        let view = verify_finalized_state_proof_v1(&self.checkpoint, bytes)
            .map_err(|_| ActorProofError::Proof)?;
        if !fresh(now, view.target_timestamp()) {
            return Err(ActorProofError::Proof);
        }
        if view
            .require_absent(
                cowboy_protocol_codec::Address::from_bytes(actor),
                ACTOR_CONTROL_KEY,
            )
            .is_ok()
        {
            return Ok(VerifiedActorState::Absent(VerifiedActorAbsence {
                actor,
                chain_id: view.chain_id(),
                chain_instance: view.chain_instance_id(),
                height: view.height(),
                block_hash: view.block_hash(),
                state_root: view.state_root(),
                timestamp: view.target_timestamp(),
            }));
        }
        let value = view
            .require_present(
                cowboy_protocol_codec::Address::from_bytes(actor),
                ACTOR_CONTROL_KEY,
            )
            .map_err(|_| ActorProofError::Proof)?;
        let (controller, commitment, authorization_generation) = parse_control(value)?;
        Ok(VerifiedActorState::Present(VerifiedActorControl {
            actor,
            chain_id: view.chain_id(),
            chain_instance: view.chain_instance_id(),
            height: view.height(),
            block_hash: view.block_hash(),
            state_root: view.state_root(),
            timestamp: view.target_timestamp(),
            controller,
            commitment,
            authorization_generation,
        }))
    }
}

fn parse_control(raw: &[u8]) -> Result<([u8; 20], [u8; 32], u64), ActorProofError> {
    use ciborium::value::Value;
    canonical::validate(raw).map_err(|_| ActorProofError::Proof)?;
    let Value::Map(fields) = ciborium::from_reader(raw).map_err(|_| ActorProofError::Proof)? else {
        return Err(ActorProofError::Proof);
    };
    if fields.len() != 3 {
        return Err(ActorProofError::Proof);
    }
    let get = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == &Value::Text(key.into()))
            .map(|(_, v)| v)
            .ok_or(ActorProofError::Proof)
    };
    let Value::Bytes(controller) = get("controller")? else {
        return Err(ActorProofError::Proof);
    };
    let Value::Bytes(commitment) = get("certificate_commitment")? else {
        return Err(ActorProofError::Proof);
    };
    let Value::Integer(generation) = get("authorization_generation")? else {
        return Err(ActorProofError::Proof);
    };
    Ok((
        controller
            .as_slice()
            .try_into()
            .map_err(|_| ActorProofError::Proof)?,
        commitment
            .as_slice()
            .try_into()
            .map_err(|_| ActorProofError::Proof)?,
        (*generation)
            .try_into()
            .map_err(|_| ActorProofError::Proof)?,
    ))
}

#[cfg(test)]
pub(crate) mod tests;
