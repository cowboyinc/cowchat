use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoomKeyState {
    pub transition_id: [u8; 32],
    pub policy_epoch: u64,
    pub key_epoch: u64,
    pub policy_hash: [u8; 32],
    pub control_root: [u8; 32],
}

/// Immutable publication bytes, archived before any control-volume write.
/// Contains public signatures and encrypted custody, never plaintext keys.
/// This is recovery intent, not an attestation or permission to publish: the
/// coordinator must verify signatures, current roots and fresh ALL-holder ACKs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoomKeyPreparation {
    pub transition_id: [u8; 32],
    pub expected_policy_hash: Option<[u8; 32]>,
    pub policy_epoch: u64,
    pub key_epoch: u64,
    pub policy_hash: [u8; 32],
    pub expected_control_root: [u8; 32],
    /// Original owner-signed setup request, lowercase canonical hex.
    pub signed_setup: String,
    /// Needed to reconstruct and verify a pending successor after its new
    /// policy has replaced the old mutable control record. Older experimental
    /// preparations may omit it; the publication verifier still requires it
    /// for successors and must fail closed if it cannot be recovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_policy: Option<String>,
    pub signed_policy: String,
    pub custody: String,
    pub grants: Vec<String>,
}

pub(super) fn transition_id(id: &[u8; 32]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

impl RoomKeyPreparation {
    pub fn command_id(&self) -> String {
        format!("prepare-{}", transition_id(&self.transition_id))
    }

    // Structural replay checks only; authentication belongs at the coordinator.
    pub(super) fn extends(&self, previous: Option<&RoomKeyState>) -> bool {
        let successor = match previous {
            None => {
                self.expected_policy_hash.is_none() && self.policy_epoch == 0 && self.key_epoch == 0
            }
            Some(previous) => {
                self.expected_policy_hash == Some(previous.policy_hash)
                    && self.policy_epoch > previous.policy_epoch
                    && self.key_epoch > previous.key_epoch
                    && self.policy_hash != previous.policy_hash
            }
        };
        successor
            && self.transition_id != [0; 32]
            && self.policy_hash != [0; 32]
            && self.expected_control_root != [0; 32]
            && hex_bytes(&self.signed_setup, 32 * 1024)
            && self.previous_policy.as_ref().is_none_or(|p| hex_bytes(p, 96 * 1024))
            && hex_bytes(&self.signed_policy, 96 * 1024)
            && self.custody.len() == 157 * 2 && hex_bytes(&self.custody, 157)
            && self.grants.len() <= 1024
            && self.grants.iter().all(|g| g.len() == 178 * 2 && hex_bytes(g, 178))
            // Leave room for the command and CBQS envelope below 256 KiB.
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= 240 * 1024)
    }

    pub(super) fn matches_commit(&self, expected: Option<[u8; 32]>, state: &RoomKeyState) -> bool {
        self.expected_policy_hash == expected
            && self.transition_id == state.transition_id
            && self.policy_epoch == state.policy_epoch
            && self.key_epoch == state.key_epoch
            && self.policy_hash == state.policy_hash
            && state.control_root != [0; 32]
            && state.control_root != self.expected_control_root
    }
}

fn hex_bytes(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes * 2
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
