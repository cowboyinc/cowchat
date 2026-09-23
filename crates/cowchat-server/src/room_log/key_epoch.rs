use serde::{Deserialize, Serialize};

/// A full setup, current/previous policies, encrypted custody and one grant per
/// retained member and key epoch must remain one CBQS record below the 256 KiB
/// wire ceiling. One-at-a-time removals from 32 members peak at 272 grants.
pub(crate) const MAX_DURABLE_ROOM_MEMBERS: usize = 32;
pub(crate) const MAX_DURABLE_ROOM_GRANTS: usize =
    (MAX_DURABLE_ROOM_MEMBERS + 1) * (MAX_DURABLE_ROOM_MEMBERS + 1) / 4;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoomKeyState {
    pub transition_id: [u8; 32],
    pub policy_epoch: u64,
    pub key_epoch: u64,
    pub policy_hash: [u8; 32],
    pub control_root: [u8; 32],
}

/// One committed encrypted room key, retained so current members can recover
/// every epoch authorized by the latest policy. The plaintext key never enters
/// the owner log.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoomKeyCustody {
    pub key_epoch: u64,
    pub custody: String,
}

/// Immutable publication bytes, archived before any control-volume write.
/// Contains public signatures and encrypted custody, never plaintext keys.
/// This is recovery intent, not an attestation or permission to publish: the
/// coordinator must verify signatures, current roots and fresh ALL-holder ACKs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoomKeyPreparation {
    #[serde(deserialize_with = "byte32::deserialize")]
    pub transition_id: [u8; 32],
    #[serde(default, deserialize_with = "byte32::deserialize_option")]
    pub expected_policy_hash: Option<[u8; 32]>,
    pub policy_epoch: u64,
    pub key_epoch: u64,
    #[serde(deserialize_with = "byte32::deserialize")]
    pub policy_hash: [u8; 32],
    #[serde(deserialize_with = "byte32::deserialize")]
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

// WASM returns fixed-width metadata as canonical hex while the durable room
// log stores serde's byte-array form. Accept both at the client boundary and
// keep the persisted representation unchanged.
mod byte32 {
    use serde::{de::Error, Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Hex(String),
        Bytes([u8; 32]),
    }

    fn decode<E: Error>(wire: Wire) -> Result<[u8; 32], E> {
        match wire {
            Wire::Bytes(bytes) => Ok(bytes),
            Wire::Hex(value)
                if value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
            {
                let mut bytes = [0; 32];
                hex::decode_to_slice(value, &mut bytes).map_err(E::custom)?;
                Ok(bytes)
            }
            Wire::Hex(_) => Err(E::custom("expected 32-byte lowercase hex")),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error> {
        decode(Wire::deserialize(deserializer)?)
    }

    pub(super) fn deserialize_option<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        Option::<Wire>::deserialize(deserializer)?
            .map(decode)
            .transpose()
    }
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
            && self.grants.len() <= MAX_DURABLE_ROOM_GRANTS
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preparation() -> RoomKeyPreparation {
        RoomKeyPreparation {
            transition_id: [1; 32],
            expected_policy_hash: Some([2; 32]),
            policy_epoch: 1,
            key_epoch: 1,
            policy_hash: [3; 32],
            expected_control_root: [4; 32],
            signed_setup: "01".into(),
            previous_policy: Some("02".into()),
            signed_policy: "03".into(),
            custody: "04".into(),
            grants: vec!["05".into()],
        }
    }

    #[test]
    fn preparation_accepts_wasm_hex_and_durable_byte_arrays() {
        let expected = preparation();
        let mut wasm = serde_json::to_value(&expected).unwrap();
        let object = wasm.as_object_mut().unwrap();
        for (field, byte) in [
            ("transition_id", 1),
            ("expected_policy_hash", 2),
            ("policy_hash", 3),
            ("expected_control_root", 4),
        ] {
            object.insert(field.into(), json!(format!("{byte:02x}").repeat(32)));
        }
        assert_eq!(
            serde_json::from_value::<RoomKeyPreparation>(wasm.clone()).unwrap(),
            expected
        );
        wasm["expected_policy_hash"] = serde_json::Value::Null;
        let mut initial = expected.clone();
        initial.expected_policy_hash = None;
        assert_eq!(
            serde_json::from_value::<RoomKeyPreparation>(wasm).unwrap(),
            initial
        );
        assert_eq!(
            serde_json::from_value::<RoomKeyPreparation>(serde_json::to_value(&expected).unwrap())
                .unwrap(),
            expected
        );
    }

    #[test]
    fn durable_member_limit_keeps_a_successor_preparation_below_the_record_cap() {
        let previous = RoomKeyState {
            transition_id: [9; 32],
            policy_epoch: 0,
            key_epoch: 0,
            policy_hash: [2; 32],
            control_root: [8; 32],
        };
        let mut value = preparation();
        value.signed_setup = "01".repeat(16 * 1024);
        value.previous_policy = Some("02".repeat(16 * 1024));
        value.signed_policy = "03".repeat(16 * 1024);
        value.custody = "04".repeat(157);
        value.grants = vec!["05".repeat(178); MAX_DURABLE_ROOM_GRANTS];
        assert!(serde_json::to_vec(&value).unwrap().len() < 240 * 1024);
        assert!(value.extends(Some(&previous)));

        value.grants.push("05".repeat(178));
        assert!(!value.extends(Some(&previous)));
    }

    #[test]
    fn preparation_rejects_noncanonical_wasm_hex() {
        for invalid in ["01".to_string(), "AA".repeat(32)] {
            let mut value = serde_json::to_value(preparation()).unwrap();
            value["policy_hash"] = json!(invalid);
            assert!(serde_json::from_value::<RoomKeyPreparation>(value).is_err());
        }
    }
}
