//! Initial authenticated hosted surface: private encrypted rooms, transient
//! connection membership, messages and reads. Other durable mutations are
//! refused here, never passed through to the local SQLite room handlers.
use crate::{
    broker::Broker,
    rate_limit::{RateLimiter, TierLimits},
    reconnect::ReconnectManager,
    room_log::{
        runtime::{OwnerRuntime, OwnerView, RuntimeError},
        Command, CommandBody, Outcome, Rejection, RoomKeyPreparation, RoomState,
    },
    store::Store,
};
use cowchat_core::*;
use serde::de::DeserializeOwned;
use std::{collections::HashSet, sync::Arc};

#[cfg(feature = "room-key-demo")]
use crate::room_log::RoomKeyCustody;

#[cfg(feature = "room-key-demo")]
use commonware_codec::Decode;
#[cfg(feature = "room-key-demo")]
use cowboy_protocol_codec::{
    room_policy::SignedRoomKeyPolicyV1, room_release::SignedRoomKeyGrantV1,
    room_setup::SignedRoomSetupV1, room_transport::RoomKeyCallV1, Address,
};

#[cfg(feature = "room-key-demo")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoomKeyActivationTransition {
    Committed,
    ResumePending,
    Stage,
}

#[cfg(feature = "room-key-demo")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRoomKeyPayload {
    room_id: String,
    name: String,
    owner: String,
    #[serde(default)]
    remove_member: Option<String>,
}

#[cfg(all(test, feature = "room-key-demo"))]
mod member_policy_tests {
    use super::*;
    use crate::{
        broker::Broker, connection::AgentConnection, hosted_bootstrap::BrowserInitialRoom,
        reconnect::ReconnectManager, room_log::OwnerState,
    };
    use commonware_codec::Encode;
    use cowboy_protocol_codec::{
        room_policy::{
            RoomCustodyCommitmentV1, RoomKeyPolicyV1, RoomPolicyIdentityV1, SignedRoomKeyPolicyV1,
        },
        room_release::{RoomKeyGrantV1, SignedRoomKeyGrantV1},
        EthSignature,
    };
    use cowchat_core::AgentInfo;
    use dashmap::DashMap;
    use k256::ecdsa::SigningKey;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn signed_policy(
        owner: &SigningKey,
        policy_epoch: u64,
        active_key_epoch: u64,
        mut members: Vec<Address>,
        keys: Vec<RoomCustodyCommitmentV1>,
    ) -> SignedRoomKeyPolicyV1 {
        members.sort_unstable();
        let policy = RoomKeyPolicyV1 {
            identity: RoomPolicyIdentityV1 {
                chain_id: 1,
                chain_instance_id: [1; 32],
                service_id: [2; 32],
                owner: Address::from_verifying_key(owner.verifying_key()),
                room_id: "11111111-1111-4111-8111-111111111111".into(),
            },
            policy_epoch,
            active_key_epoch,
            members,
            keys,
        };
        SignedRoomKeyPolicyV1 {
            owner_signature: EthSignature::sign(owner, &policy.signing_hash()),
            policy,
        }
    }

    fn connection(
        id: &str,
        api_key: &str,
        member_address: Option<String>,
    ) -> (AgentConnection, mpsc::Receiver<Frame>) {
        let (sender, receiver) = mpsc::channel(16);
        (
            AgentConnection::new_with_member(
                AgentInfo {
                    agent_id: id.into(),
                    name: id.into(),
                    capabilities: vec![],
                    connected_at: None,
                    last_active: None,
                    status: None,
                    status_detail: None,
                    progress: None,
                },
                format!("session-{id}"),
                sender,
                tokio::spawn(async {}),
                tokio::spawn(async {}),
                Arc::new(tokio::sync::Notify::new()),
                api_key.into(),
                member_address,
            ),
            receiver,
        )
    }

    fn drained(receiver: &mut mpsc::Receiver<Frame>) -> Vec<Frame> {
        let mut frames = Vec::new();
        while let Ok(frame) = receiver.try_recv() {
            frames.push(frame);
        }
        frames
    }

    fn structural_preparation() -> RoomKeyPreparation {
        RoomKeyPreparation {
            transition_id: [1; 32],
            expected_policy_hash: None,
            policy_epoch: 0,
            key_epoch: 0,
            policy_hash: [2; 32],
            expected_control_root: [3; 32],
            signed_setup: "01".into(),
            previous_policy: None,
            signed_policy: "02".into(),
            custody: "03".repeat(157),
            grants: vec!["04".repeat(178)],
        }
    }

    #[test]
    fn hosted_member_access_comes_only_from_the_signed_roster() {
        let owner = SigningKey::from_bytes((&[0x11; 32]).into()).unwrap();
        let member = SigningKey::from_bytes((&[0x22; 32]).into()).unwrap();
        let member_address = Address::from_verifying_key(member.verifying_key());
        let signed = signed_policy(
            &owner,
            0,
            0,
            vec![
                Address::from_verifying_key(owner.verifying_key()),
                member_address,
            ],
            vec![RoomCustodyCommitmentV1 {
                key_epoch: 0,
                scope_hash: [3; 32],
                ciphertext_hash: [4; 32],
            }],
        );
        let bytes = hex::encode(signed.encode());
        let address = format!("0x{}", hex::encode(member_address.as_bytes()));
        assert!(signed_policy_has_member(
            &bytes,
            "11111111-1111-4111-8111-111111111111",
            &address
        ));
        assert!(!signed_policy_has_member(
            &bytes,
            "22222222-2222-4222-8222-222222222222",
            &address
        ));
        assert!(!signed_policy_has_member(
            &bytes,
            "11111111-1111-4111-8111-111111111111",
            &format!("0x{}", "44".repeat(20))
        ));
        let mut successor = signed.policy;
        successor.policy_epoch = 1;
        successor.active_key_epoch = 1;
        successor
            .members
            .retain(|candidate| *candidate != member_address);
        successor.keys.push(RoomCustodyCommitmentV1 {
            key_epoch: 1,
            scope_hash: [5; 32],
            ciphertext_hash: [6; 32],
        });
        let successor = SignedRoomKeyPolicyV1 {
            owner_signature: EthSignature::sign(&owner, &successor.signing_hash()),
            policy: successor,
        };
        assert!(!signed_policy_has_member(
            &hex::encode(successor.encode()),
            "11111111-1111-4111-8111-111111111111",
            &address
        ));
    }

    #[test]
    fn room_context_selects_all_member_epochs_or_fails_as_one_batch() {
        let owner = SigningKey::from_bytes((&[0x11; 32]).into()).unwrap();
        let member = SigningKey::from_bytes((&[0x22; 32]).into()).unwrap();
        let outsider = SigningKey::from_bytes((&[0x33; 32]).into()).unwrap();
        let owner_address = Address::from_verifying_key(owner.verifying_key());
        let member_address = Address::from_verifying_key(member.verifying_key());
        let policy = signed_policy(
            &owner,
            2,
            2,
            vec![member_address],
            vec![
                RoomCustodyCommitmentV1 {
                    key_epoch: 0,
                    scope_hash: [3; 32],
                    ciphertext_hash: [4; 32],
                },
                RoomCustodyCommitmentV1 {
                    key_epoch: 1,
                    scope_hash: [5; 32],
                    ciphertext_hash: [6; 32],
                },
                RoomCustodyCommitmentV1 {
                    key_epoch: 2,
                    scope_hash: [7; 32],
                    ciphertext_hash: [8; 32],
                },
            ],
        );
        let encoded = [
            (2, [7; 32], [8; 32]),
            (0, [3; 32], [4; 32]),
            (1, [5; 32], [6; 32]),
        ]
        .into_iter()
        .map(|(_, scope_hash, ciphertext_hash)| {
            let grant = RoomKeyGrantV1 {
                owner: owner_address,
                member: member_address,
                scope_hash,
                ciphertext_hash,
                policy_epoch: 2,
            };
            hex::encode(
                SignedRoomKeyGrantV1 {
                    owner_signature: EthSignature::sign(&owner, &grant.signing_hash()),
                    grant,
                }
                .encode(),
            )
        })
        .collect::<Vec<_>>();

        let grants = member_grants(&policy, &encoded, member_address).unwrap();
        assert_eq!(
            grants.iter().map(|(epoch, _)| *epoch).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(grants.last().unwrap().1.grant.scope_hash, [7; 32]);
        assert!(member_grants(
            &policy,
            &encoded,
            Address::from_verifying_key(outsider.verifying_key())
        )
        .is_none());

        let custodies = (0..3)
            .map(|key_epoch| RoomKeyCustody {
                key_epoch,
                custody: format!("{:02x}", key_epoch + 10).repeat(157),
            })
            .collect::<Vec<_>>();
        let contexts = member_key_contexts(grants.clone(), &custodies).unwrap();
        assert_eq!(
            contexts
                .iter()
                .map(|context| context.key_epoch)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(contexts
            .iter()
            .all(|context| context.custody_bytes.len() == 157));

        assert!(member_key_contexts(grants.clone(), &custodies[..2]).is_none());
        let mut corrupt = custodies.clone();
        corrupt[1].custody = "zz".repeat(157);
        assert!(member_key_contexts(grants.clone(), &corrupt).is_none());
        let mut out_of_order = custodies;
        out_of_order.swap(0, 1);
        assert!(member_key_contexts(grants, &out_of_order).is_none());
    }

    #[tokio::test]
    async fn successor_commit_evicts_removed_wallet_and_fans_out_without_leaking() {
        let owner = SigningKey::from_bytes((&[0x11; 32]).into()).unwrap();
        let removed = SigningKey::from_bytes((&[0x22; 32]).into()).unwrap();
        let retained = SigningKey::from_bytes((&[0x33; 32]).into()).unwrap();
        let outsider = SigningKey::from_bytes((&[0x44; 32]).into()).unwrap();
        let owner_address = Address::from_verifying_key(owner.verifying_key());
        let removed_address = Address::from_verifying_key(removed.verifying_key());
        let retained_address = Address::from_verifying_key(retained.verifying_key());
        let predecessor = signed_policy(
            &owner,
            0,
            0,
            vec![owner_address, removed_address, retained_address],
            vec![RoomCustodyCommitmentV1 {
                key_epoch: 0,
                scope_hash: [3; 32],
                ciphertext_hash: [4; 32],
            }],
        );
        let successor = signed_policy(
            &owner,
            1,
            1,
            vec![owner_address, retained_address],
            vec![
                RoomCustodyCommitmentV1 {
                    key_epoch: 0,
                    scope_hash: [3; 32],
                    ciphertext_hash: [4; 32],
                },
                RoomCustodyCommitmentV1 {
                    key_epoch: 1,
                    scope_hash: [5; 32],
                    ciphertext_hash: [6; 32],
                },
            ],
        );
        let agents = Arc::new(DashMap::new());
        let broker = Broker::new(agents.clone(), Arc::new(DashMap::new()));
        let mut receivers = std::collections::HashMap::new();
        for (id, api_key, member) in [
            (
                "removed",
                "",
                Some(format!("0x{}", hex::encode(removed_address.as_bytes()))),
            ),
            (
                "removed-idle",
                "",
                Some(format!("0x{}", hex::encode(removed_address.as_bytes()))),
            ),
            (
                "retained",
                "",
                Some(format!("0x{}", hex::encode(retained_address.as_bytes()))),
            ),
            (
                "outsider",
                "",
                Some(format!(
                    "0x{}",
                    hex::encode(Address::from_verifying_key(outsider.verifying_key()).as_bytes())
                )),
            ),
            ("service", "service-key", None),
            ("foreign-service", "foreign-key", None),
        ] {
            let (connection, receiver) = connection(id, api_key, member);
            agents.insert(id.into(), connection);
            receivers.insert(id, receiver);
        }
        for id in ["removed", "service", "retained"] {
            broker
                .join_room(id, &predecessor.policy.identity.room_id, || true)
                .unwrap();
        }
        assert_eq!(
            broker
                .turn_holder(&predecessor.policy.identity.room_id)
                .as_deref(),
            Some("removed")
        );

        let room = Room {
            room_id: predecessor.policy.identity.room_id.clone(),
            name: "room".into(),
            description: None,
            parent_id: None,
            created_at: chrono::Utc::now(),
            created_by: Some("owner".into()),
            visibility: "private".into(),
            owner_key: None,
            last_activity: None,
            member_count: None,
            encrypted: true,
        };
        let reconnect = ReconnectManager::new();
        // A stale reconnect stash must not suppress the event for a live
        // service session using the same stable agent ID.
        reconnect.stash(
            "service".into(),
            "service".into(),
            "service-key".into(),
            HashSet::new(),
        );
        reconcile_room_key_commit(
            &broker,
            &reconnect,
            "service-key",
            &room,
            Some(&predecessor),
            &successor,
        );

        assert!(broker.agents.contains_key("removed"));
        assert!(!broker.is_agent_in_room("removed", &room.room_id));
        assert!(broker.is_agent_in_room("service", &room.room_id));
        assert!(broker.is_agent_in_room("retained", &room.room_id));
        assert_eq!(
            broker.turn_holder(&room.room_id).as_deref(),
            Some("service")
        );

        for id in ["removed", "removed-idle", "retained", "service"] {
            assert!(drained(receivers.get_mut(id).unwrap())
                .iter()
                .any(|frame| frame.frame_type == FrameType::RoomUpdated));
        }
        for id in ["outsider", "foreign-service"] {
            assert!(!drained(receivers.get_mut(id).unwrap())
                .iter()
                .any(|frame| frame.frame_type == FrameType::RoomUpdated));
        }

        reconcile_room_key_commit(
            &broker,
            &reconnect,
            "service-key",
            &room,
            Some(&predecessor),
            &successor,
        );
        let replay = drained(receivers.get_mut("retained").unwrap());
        assert!(replay
            .iter()
            .any(|frame| frame.frame_type == FrameType::RoomUpdated));
        assert!(!replay
            .iter()
            .any(|frame| frame.frame_type == FrameType::TurnChanged));
        assert_eq!(
            broker.turn_holder(&room.room_id).as_deref(),
            Some("service")
        );
    }

    #[test]
    fn exact_committed_activation_is_classified_locally() {
        let room_id = "11111111-1111-4111-8111-111111111111";
        let preparation = structural_preparation();
        let mut state = OwnerState::new("owner".into());
        let create = Command {
            owner_id: "owner".into(),
            command_id: "create".into(),
            timestamp: chrono::Utc::now(),
            body: CommandBody::CreateRoom {
                room_id: room_id.into(),
                lane_id: 7,
                name: "room".into(),
                created_by: "owner".into(),
            },
        };
        let prepare = Command {
            owner_id: "owner".into(),
            command_id: preparation.command_id(),
            timestamp: chrono::Utc::now(),
            body: CommandBody::PrepareKeyEpoch {
                room_id: room_id.into(),
                preparation: Box::new(preparation.clone()),
            },
        };
        let commit = Command {
            owner_id: "owner".into(),
            command_id: hex::encode(preparation.transition_id),
            timestamp: chrono::Utc::now(),
            body: CommandBody::CommitKeyEpoch {
                room_id: room_id.into(),
                expected_policy_hash: None,
                state: crate::room_log::RoomKeyState {
                    transition_id: preparation.transition_id,
                    policy_epoch: 0,
                    key_epoch: 0,
                    policy_hash: preparation.policy_hash,
                    control_root: [4; 32],
                },
            },
        };
        for (sequence, command) in [(1, create), (2, prepare), (3, commit)] {
            assert!(!matches!(
                state.apply(sequence, 0, &command).unwrap(),
                Outcome::Rejected { .. }
            ));
        }
        let owner = Address::from_bytes([9; 20]);
        let input = BrowserInitialRoom {
            room_id: room_id.into(),
            name: "room".into(),
            created_by: "owner".into(),
            room_owner: owner,
            preparation,
        };
        assert_eq!(
            classify_room_key_activation(&state, &input, "room", owner, 50, (5, 2), None).unwrap(),
            RoomKeyActivationTransition::Committed
        );
        let mut changed = input.clone();
        changed.preparation.signed_policy.push_str("00");
        assert!(
            classify_room_key_activation(&state, &changed, "room", owner, 50, (5, 2), None)
                .is_err()
        );
    }
}

#[cfg(feature = "room-key-demo")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomKeyBytesPayload {
    request: String,
}

#[cfg(feature = "room-key-demo")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomKeyContextPayload {
    room_id: String,
    member: String,
}

#[cfg(feature = "room-key-demo")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateRoomKeyPayload {
    room_id: String,
    name: String,
    preparation: RoomKeyPreparation,
}

#[cfg(feature = "room-key-demo")]
fn bounded_hex(value: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.is_empty() || value.len() > max_bytes * 2 || !value.len().is_multiple_of(2) {
        return None;
    }
    hex::decode(value).ok()
}

#[cfg(feature = "room-key-demo")]
fn parse_address(value: &str) -> Option<Address> {
    bounded_hex(value, 20)?
        .try_into()
        .ok()
        .map(Address::from_bytes)
        .filter(|address| *address != Address::ZERO)
}

pub struct HostedOwner {
    owner_id: String,
    api_key: String,
    writer: tokio::sync::Mutex<OwnerRuntime>,
    view: Arc<OwnerView>,
    #[cfg(feature = "room-key-demo")]
    room_keys: Option<crate::hosted_bootstrap::HostedRoomKeys>,
    /// Serializes control-volume publication for this owner without holding
    /// the owner-log writer across CBSS finality. Two room activations built
    /// from the same control root must not race their compare-and-swap writes.
    #[cfg(feature = "room-key-demo")]
    room_key_activation: tokio::sync::Mutex<()>,
}

fn error(id: Option<&str>, code: ErrorCode, message: &str) -> Frame {
    Frame::error(id, ErrorPayload::new(code, message))
}
fn unavailable(id: Option<&str>) -> Frame {
    error(
        id,
        ErrorCode::InternalError,
        "Hosted owner is unavailable; retain the prepared request for retry",
    )
}
fn parse<T: DeserializeOwned>(frame: &Frame) -> Result<T, Frame> {
    serde_json::from_value(frame.payload.clone()).map_err(|_| {
        error(
            frame.id.as_deref(),
            ErrorCode::InvalidPayload,
            "Invalid request payload",
        )
    })
}
fn response(frame: &Frame, kind: FrameType, payload: serde_json::Value) -> Frame {
    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: frame.id.clone(),
        frame_type: kind,
        payload,
    }
}
fn rejected(id: Option<&str>, outcome: &Outcome) -> Result<(), Frame> {
    if let Outcome::Rejected { reason } = outcome {
        let code = match reason {
            Rejection::RoomNameTaken => ErrorCode::RoomNameTaken,
            Rejection::UnknownRoom => ErrorCode::RoomNotFound,
            Rejection::Plaintext => ErrorCode::PlaintextInEncryptedRoom,
            Rejection::CommandConflict => ErrorCode::MessageConflict,
            _ => ErrorCode::InvalidPayload,
        };
        Err(error(
            id,
            code,
            "Command conflicts with the committed room state",
        ))
    } else {
        Ok(())
    }
}
fn room_summary(room: &RoomState) -> Room {
    Room {
        room_id: room.room_id.clone(),
        name: room.name.clone(),
        description: None,
        parent_id: None,
        created_at: room.created_at,
        created_by: Some(room.created_by.clone()),
        visibility: "private".into(),
        owner_key: None,
        last_activity: room.messages.last().map(|m| m.message.timestamp),
        member_count: None,
        encrypted: true,
    }
}

#[cfg(feature = "room-key-demo")]
fn signed_policy_has_member(signed_policy: &str, room_id: &str, address: &str) -> bool {
    let Some(member) = parse_address(address) else {
        return false;
    };
    let Some(bytes) = bounded_hex(signed_policy, 96 * 1024) else {
        return false;
    };
    let Ok(policy) = SignedRoomKeyPolicyV1::decode_canonical(&bytes) else {
        return false;
    };
    policy.verify_owner().is_ok()
        && policy.policy.identity.room_id == room_id
        && policy.policy.members.binary_search(&member).is_ok()
}

#[cfg(feature = "room-key-demo")]
fn room_has_member(room: &RoomState, address: &str) -> bool {
    room.key_publication.as_ref().is_some_and(|publication| {
        signed_policy_has_member(&publication.signed_policy, &room.room_id, address)
    })
}

#[cfg(feature = "room-key-demo")]
fn member_grants(
    policy: &SignedRoomKeyPolicyV1,
    encoded_grants: &[String],
    member: Address,
) -> Option<Vec<(u64, SignedRoomKeyGrantV1)>> {
    policy.verify_owner().ok()?;
    let mut grants = encoded_grants
        .iter()
        .filter_map(|value| bounded_hex(value, 178))
        .filter_map(|bytes| SignedRoomKeyGrantV1::decode_cfg(bytes.as_slice(), &()).ok())
        .filter_map(|grant| {
            if grant.grant.member != member
                || !grant
                    .owner_signature
                    .recover_address(&grant.grant.signing_hash())
                    .is_ok_and(|signer| signer == policy.policy.identity.owner)
            {
                return None;
            }
            policy
                .policy
                .match_grant(&grant.grant)
                .ok()
                .map(|key| (key.key_epoch, grant))
        })
        .collect::<Vec<_>>();
    grants.sort_unstable_by_key(|(key_epoch, _)| *key_epoch);
    (grants.len() == policy.policy.keys.len()
        && grants
            .iter()
            .zip(&policy.policy.keys)
            .all(|((granted_epoch, _), key)| *granted_epoch == key.key_epoch))
    .then_some(grants)
}

#[cfg(feature = "room-key-demo")]
struct MemberKeyContext<'a> {
    key_epoch: u64,
    grant: SignedRoomKeyGrantV1,
    custody: &'a str,
    custody_bytes: Vec<u8>,
}

#[cfg(feature = "room-key-demo")]
fn member_key_contexts<'a>(
    grants: Vec<(u64, SignedRoomKeyGrantV1)>,
    custodies: &'a [RoomKeyCustody],
) -> Option<Vec<MemberKeyContext<'a>>> {
    if grants.len() != custodies.len() {
        return None;
    }
    grants
        .into_iter()
        .zip(custodies)
        .map(|((key_epoch, grant), custody)| {
            if custody.key_epoch != key_epoch || custody.custody.len() != 157 * 2 {
                return None;
            }
            let custody_bytes = bounded_hex(&custody.custody, 157)?;
            (custody_bytes.len() == 157).then_some(MemberKeyContext {
                key_epoch,
                grant,
                custody: &custody.custody,
                custody_bytes,
            })
        })
        .collect()
}

#[cfg(feature = "room-key-demo")]
fn classify_room_key_activation(
    state: &crate::room_log::OwnerState,
    input: &crate::hosted_bootstrap::BrowserInitialRoom,
    normalized_name: &str,
    room_owner: Address,
    max_rooms: u64,
    wallet_limits: (usize, usize),
    id: Option<&str>,
) -> Result<RoomKeyActivationTransition, Frame> {
    match state.room(&input.room_id) {
        Some(existing)
            if existing.name == normalized_name
                && existing.key_publication.as_deref() == Some(&input.preparation)
                && existing.key_state.is_some() =>
        {
            Ok(RoomKeyActivationTransition::Committed)
        }
        Some(existing)
            if existing.name == normalized_name
                && existing.key_preparation.as_deref() == Some(&input.preparation)
                && existing.key_publication.as_deref() != Some(&input.preparation) =>
        {
            Ok(RoomKeyActivationTransition::ResumePending)
        }
        Some(existing)
            if existing.name == normalized_name
                && existing.key_preparation.is_none()
                && input.preparation.expected_policy_hash.is_some() =>
        {
            Ok(RoomKeyActivationTransition::Stage)
        }
        Some(_) => Err(error(
            id,
            ErrorCode::MessageConflict,
            "Room activation conflicts with committed state",
        )),
        None if input.preparation.expected_policy_hash.is_some() => Err(error(
            id,
            ErrorCode::MessageConflict,
            "Room activation conflicts with committed state",
        )),
        None if state.rooms().len() as u64 >= max_rooms => {
            Err(error(id, ErrorCode::RateLimitRooms, "Room limit exceeded"))
        }
        None => {
            let (active, pending) = wallet_room_usage(state, room_owner);
            if active + pending >= wallet_limits.0 || pending >= wallet_limits.1 {
                Err(error(
                    id,
                    ErrorCode::RateLimitRooms,
                    "Wallet room limit exceeded",
                ))
            } else {
                Ok(RoomKeyActivationTransition::Stage)
            }
        }
    }
}

#[cfg(feature = "room-key-demo")]
fn reconcile_room_key_commit(
    broker: &Broker,
    reconnect: &ReconnectManager,
    service_api_key: &str,
    room: &Room,
    previous: Option<&SignedRoomKeyPolicyV1>,
    current: &SignedRoomKeyPolicyV1,
) {
    let event = Frame::event(
        if previous.is_some() {
            FrameType::RoomUpdated
        } else {
            FrameType::RoomCreated
        },
        serde_json::to_value(room).unwrap(),
    );
    let _lifecycle = broker.lock_agent_lifecycle();
    let _ = reconnect.buffer_visible_room_event(
        "private",
        Some(service_api_key),
        &HashSet::new(),
        false,
        &event,
    );
    let notification_policy = previous.unwrap_or(current);
    let live = broker
        .agents
        .iter()
        .map(|agent| {
            let member = agent.member_address.as_deref().and_then(parse_address);
            let is_wallet = agent.member_address.is_some();
            let retained =
                member.is_some_and(|member| current.policy.members.binary_search(&member).is_ok());
            let relevant = if is_wallet {
                member.is_some_and(|member| {
                    notification_policy
                        .policy
                        .members
                        .binary_search(&member)
                        .is_ok()
                })
            } else {
                agent.api_key == service_api_key
            };
            (agent.key().clone(), is_wallet, retained, relevant)
        })
        .collect::<Vec<_>>();

    let holder_before = broker.turn_holder(&room.room_id);
    for (agent_id, is_wallet, retained, _) in &live {
        if *is_wallet && !retained && broker.is_agent_in_room(agent_id, &room.room_id) {
            broker.leave_room(agent_id, &room.room_id);
            broker.broadcast_to_room_all(
                &room.room_id,
                &Frame::event(
                    FrameType::AgentLeft,
                    serde_json::json!({"room_id":room.room_id,"agent_id":agent_id}),
                ),
            );
        }
    }
    if holder_before != broker.turn_holder(&room.room_id) {
        crate::handler::broadcast_turn_changed(broker, &room.room_id, "left");
    }
    for (agent_id, _, _, relevant) in live {
        if relevant {
            broker.send_to_agent(&agent_id, event.clone());
        }
    }
}

#[cfg(feature = "room-key-demo")]
fn wallet_room_usage(state: &crate::room_log::OwnerState, owner: Address) -> (usize, usize) {
    let mut active = 0;
    let mut pending = 0;
    for room in state.rooms().values() {
        if let Some(publication) = &room.key_publication {
            let matches = bounded_hex(&publication.signed_policy, 96 * 1024)
                .and_then(|bytes| SignedRoomKeyPolicyV1::decode_canonical(&bytes).ok())
                .is_some_and(|policy| {
                    policy.verify_owner().is_ok() && policy.policy.identity.owner == owner
                });
            if matches {
                active += 1;
            }
        } else if let Some(preparation) = &room.key_preparation {
            let matches = bounded_hex(&preparation.signed_setup, 32 * 1024)
                .and_then(|bytes| SignedRoomSetupV1::decode_canonical(&bytes).ok())
                .is_some_and(|setup| {
                    setup.verify_owner_at(setup.request.issued_at_ms).is_ok()
                        && setup.request.intent.identity.owner == owner
                });
            if matches {
                pending += 1;
            }
        }
    }
    (active, pending)
}

impl HostedOwner {
    pub(crate) fn new(runtime: OwnerRuntime, api_key: String) -> Result<Self, RuntimeError> {
        drop(runtime.state()?);
        if api_key.is_empty() {
            return Err(RuntimeError::Configuration);
        }
        Ok(Self {
            owner_id: runtime.owner_id().to_owned(),
            api_key,
            view: runtime.view(),
            writer: tokio::sync::Mutex::new(runtime),
            #[cfg(feature = "room-key-demo")]
            room_keys: None,
            #[cfg(feature = "room-key-demo")]
            room_key_activation: tokio::sync::Mutex::new(()),
        })
    }

    #[cfg(feature = "room-key-demo")]
    pub(crate) fn with_room_keys(
        mut self,
        room_keys: crate::hosted_bootstrap::HostedRoomKeys,
    ) -> Self {
        self.room_keys = Some(room_keys);
        self
    }

    /// Called under existing synchronous lifecycle guards. Reads only committed
    /// memory; it neither waits for the writer queue nor performs network I/O.
    pub(crate) fn accessible_room(
        &self,
        id: &str,
        key: &str,
        member: Option<&str>,
    ) -> Option<Room> {
        let state = self.view.read().ok()?;
        let room = state.room(id)?;
        let allowed = member.map_or(key == self.api_key, |address| {
            #[cfg(feature = "room-key-demo")]
            {
                room_has_member(room, address)
            }
            #[cfg(not(feature = "room-key-demo"))]
            {
                let _ = address;
                false
            }
        });
        allowed.then(|| room_summary(room))
    }

    pub(crate) async fn handle(
        &self,
        frame: Frame,
        agent_id: &str,
        agent_name: &str,
        key: &str,
        member_address: Option<&str>,
        broker: &Broker,
        store: &Store,
        rates: &RateLimiter,
        reconnect: &ReconnectManager,
    ) -> Frame {
        self.dispatch(
            &frame,
            agent_id,
            agent_name,
            key,
            member_address,
            broker,
            store,
            rates,
            reconnect,
        )
        .await
        .unwrap_or_else(|frame| frame)
    }

    async fn dispatch(
        &self,
        frame: &Frame,
        agent_id: &str,
        agent_name: &str,
        key: &str,
        member_address: Option<&str>,
        broker: &Broker,
        store: &Store,
        rates: &RateLimiter,
        reconnect: &ReconnectManager,
    ) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let member = if let Some(address) = member_address.filter(|_| key.is_empty()) {
            Some(address)
        } else if member_address.is_none() && key == self.api_key {
            None
        } else {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "This principal is not authorized for hosted transport",
            ));
        };
        // Reject retired owners even for methods that only touch Broker state.
        drop(self.view.read().map_err(|_| unavailable(id))?);
        if let Some(address) = member {
            self.authorize_member_frame(frame, address)?;
        }
        match frame.frame_type {
            FrameType::Ping => Ok(Frame::pong(id)),
            FrameType::CreateRoom => self.create(frame, agent_id, broker, store, reconnect).await,
            #[cfg(feature = "room-key-demo")]
            FrameType::PrepareRoomKey => self.prepare_room_key(frame, member).await,
            #[cfg(feature = "room-key-demo")]
            FrameType::AttestRoomKeySetup => self.attest_room_key_setup(frame, member).await,
            #[cfg(feature = "room-key-demo")]
            FrameType::ActivateRoomKey => {
                self.activate_room_key(frame, agent_id, member, broker, store, reconnect)
                    .await
            }
            #[cfg(feature = "room-key-demo")]
            FrameType::GetRoomKeyContext => self.room_key_context(frame, member),
            #[cfg(feature = "room-key-demo")]
            FrameType::RelayRoomKeyOpen => self.relay_room_key_open(frame).await,
            FrameType::SendMessage => {
                self.send(frame, agent_id, agent_name, broker, store, rates)
                    .await
            }
            FrameType::JoinRoom => {
                let p: JoinRoomPayload = parse(frame)?;
                self.require_room(id, &p.room_id)?;
                let was_member = broker.is_agent_in_room(agent_id, &p.room_id);
                let joined = broker
                    .join_room(agent_id, &p.room_id, || {
                        self.accessible_room(&p.room_id, key, member).is_some()
                    })
                    .map_err(|_| unavailable(id))?;
                if !was_member {
                    let event = Frame::event(
                        FrameType::AgentJoined,
                        serde_json::json!({"room_id":p.room_id,"agent":{"agent_id":agent_id,"name":agent_name}}),
                    );
                    broker.broadcast_to_room(&p.room_id, agent_id, &event);
                }
                if joined.new_holder.is_some() {
                    crate::handler::broadcast_turn_changed(broker, &p.room_id, "agent_joined");
                }
                Ok(Frame::ok(id, serde_json::json!({"room_id":p.room_id})))
            }
            FrameType::LeaveRoom => {
                let p: LeaveRoomPayload = parse(frame)?;
                self.require_room(id, &p.room_id)?;
                if !broker.is_agent_in_room(agent_id, &p.room_id) {
                    return Err(error(id, ErrorCode::NotInRoom, "Not in this room"));
                }
                let left = broker.leave_room(agent_id, &p.room_id);
                broker.broadcast_to_room(
                    &p.room_id,
                    agent_id,
                    &Frame::event(
                        FrameType::AgentLeft,
                        serde_json::json!({"room_id":p.room_id,"agent_id":agent_id}),
                    ),
                );
                if left.holder_changed {
                    crate::handler::broadcast_turn_changed(broker, &p.room_id, "agent_left");
                }
                Ok(Frame::ok(id, serde_json::json!({"room_id":p.room_id})))
            }
            FrameType::GetHistory => {
                let p: GetHistoryPayload = parse(frame)?;
                let state = self.view.read().map_err(|_| unavailable(id))?;
                let room = state
                    .room(&p.room_id)
                    .ok_or_else(|| error(id, ErrorCode::RoomNotFound, "Room not found"))?;
                let count = p.limit.min(1000) as usize;
                let floor = p.since_seq.or_else(|| {
                    p.since.as_ref().map(|since| {
                        room.messages
                            .iter()
                            .find(|m| m.message.message_id == *since)
                            .map_or(i64::MAX, |m| m.message.seq)
                    })
                });
                let messages: Vec<ChatMessage> = if let Some(floor) = floor {
                    room.messages
                        .iter()
                        .filter(|m| m.message.seq > floor)
                        .take(count)
                        .map(|m| m.message.clone())
                        .collect()
                } else {
                    let mut tail: Vec<_> = room
                        .messages
                        .iter()
                        .rev()
                        .filter(|m| p.before.is_none_or(|t| m.message.timestamp < t))
                        .take(count)
                        .map(|m| m.message.clone())
                        .collect();
                    tail.reverse();
                    tail
                };
                Ok(response(
                    frame,
                    FrameType::HistoryResult,
                    serde_json::json!({"room_id":p.room_id,"messages":messages}),
                ))
            }
            FrameType::RoomTip => {
                let p: RoomTipPayload = parse(frame)?;
                let state = self.view.read().map_err(|_| unavailable(id))?;
                let room = state
                    .room(&p.room_id)
                    .ok_or_else(|| error(id, ErrorCode::RoomNotFound, "Room not found"))?;
                Ok(response(
                    frame,
                    FrameType::RoomTipResult,
                    serde_json::json!({"room_id":p.room_id,"seq":room.messages.last().map_or(0, |m|m.message.seq)}),
                ))
            }
            FrameType::ListRooms => {
                let p: ListRoomsPayload = parse(frame)?;
                let mut rooms: Vec<_> = {
                    let state = self.view.read().map_err(|_| unavailable(id))?;
                    if p.parent_id.is_some() {
                        Vec::new()
                    } else {
                        state
                            .rooms()
                            .values()
                            .filter(|room| {
                                member.is_none_or(|address| {
                                    #[cfg(feature = "room-key-demo")]
                                    {
                                        room_has_member(room, address)
                                    }
                                    #[cfg(not(feature = "room-key-demo"))]
                                    {
                                        let _ = (room, address);
                                        false
                                    }
                                })
                            })
                            .map(room_summary)
                            .collect()
                    }
                };
                for room in &mut rooms {
                    room.member_count = Some(broker.get_room_members(&room.room_id).len());
                }
                Ok(response(
                    frame,
                    FrameType::RoomList,
                    serde_json::json!({"rooms":rooms}),
                ))
            }
            FrameType::RoomInfo => {
                let p: RoomInfoPayload = parse(frame)?;
                let room = self.require_room(id, &p.room_id)?;
                let members = broker.get_room_members(&p.room_id);
                let agents: Vec<_> = members
                    .iter()
                    .filter_map(|member| broker.agents.get(member).map(|a| a.info.clone()))
                    .collect();
                Ok(response(
                    frame,
                    FrameType::RoomInfoResult,
                    serde_json::json!({"room":room,"agents":agents,"sub_rooms":[],
                    "current_turn_holder":broker.turn_holder(&p.room_id),"turn_order":members}),
                ))
            }
            FrameType::ListAgents => {
                let p: ListAgentsPayload = parse(frame)?;
                if let Some(room) = &p.room_id {
                    self.require_room(id, room)?;
                }
                let agents: Vec<_> = broker
                    .agents
                    .iter()
                    .filter(|a| member.is_some() || a.api_key == self.api_key)
                    .filter(|a| {
                        p.room_id
                            .as_ref()
                            .is_none_or(|room| broker.is_agent_in_room(a.key(), room))
                    })
                    .map(|a| a.info.clone())
                    .collect();
                Ok(response(
                    frame,
                    FrameType::AgentList,
                    serde_json::json!({"agents":agents}),
                ))
            }
            _ => Err(error(
                id,
                ErrorCode::UnsupportedProtocol,
                "This operation is not enabled in hosted mode",
            )),
        }
    }

    fn authorize_member_frame(&self, frame: &Frame, address: &str) -> Result<(), Frame> {
        let id = frame.id.as_deref();
        match frame.frame_type {
            FrameType::Ping
            | FrameType::ListRooms
            | FrameType::PrepareRoomKey
            | FrameType::AttestRoomKeySetup
            | FrameType::ActivateRoomKey => Ok(()),
            FrameType::JoinRoom
            | FrameType::LeaveRoom
            | FrameType::GetHistory
            | FrameType::RoomTip
            | FrameType::RoomInfo
            | FrameType::SendMessage
            | FrameType::GetRoomKeyContext => {
                let room_id = frame
                    .payload
                    .get("room_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room request"))?;
                self.require_member_room(id, room_id, address)
            }
            FrameType::ListAgents => {
                let room_id = frame
                    .payload
                    .get("room_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        error(
                            id,
                            ErrorCode::AccessDenied,
                            "Member agent listing requires a room",
                        )
                    })?;
                self.require_member_room(id, room_id, address)
            }
            FrameType::RelayRoomKeyOpen => {
                let payload: RoomKeyBytesPayload = parse(frame)?;
                let bytes = bounded_hex(&payload.request, 32 * 1024).ok_or_else(|| {
                    error(id, ErrorCode::InvalidPayload, "Invalid room-key request")
                })?;
                let call = RoomKeyCallV1::decode_canonical(&bytes).map_err(|_| {
                    error(id, ErrorCode::InvalidPayload, "Invalid room-key request")
                })?;
                self.require_member_room(id, &call.identity.room_id, address)
            }
            _ => Err(error(
                id,
                ErrorCode::AccessDenied,
                "This operation requires transport authority",
            )),
        }
    }

    #[cfg(feature = "room-key-demo")]
    fn require_member_room(
        &self,
        id: Option<&str>,
        room_id: &str,
        address: &str,
    ) -> Result<(), Frame> {
        let state = self.view.read().map_err(|_| unavailable(id))?;
        let room = state
            .room(room_id)
            .ok_or_else(|| error(id, ErrorCode::RoomNotFound, "Room not found"))?;
        if !room_has_member(room, address) {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "Wallet is not a room member",
            ));
        }
        Ok(())
    }

    #[cfg(not(feature = "room-key-demo"))]
    fn require_member_room(
        &self,
        id: Option<&str>,
        _room_id: &str,
        _address: &str,
    ) -> Result<(), Frame> {
        Err(error(
            id,
            ErrorCode::AccessDenied,
            "Member sessions are unavailable",
        ))
    }

    fn require_room(&self, id: Option<&str>, room: &str) -> Result<Room, Frame> {
        let state = self.view.read().map_err(|_| unavailable(id))?;
        state
            .room(room)
            .map(room_summary)
            .ok_or_else(|| error(id, ErrorCode::RoomNotFound, "Room not found"))
    }

    #[cfg(feature = "room-key-demo")]
    fn room_keys(
        &self,
        id: Option<&str>,
    ) -> Result<&crate::hosted_bootstrap::HostedRoomKeys, Frame> {
        self.room_keys.as_ref().ok_or_else(|| {
            error(
                id,
                ErrorCode::UnsupportedProtocol,
                "Automatic room keys are unavailable",
            )
        })
    }

    #[cfg(feature = "room-key-demo")]
    async fn prepare_room_key(&self, frame: &Frame, member: Option<&str>) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let payload: PrepareRoomKeyPayload = parse(frame)?;
        let normalized_name = crate::store::normalize_room_name(&payload.name)
            .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room key request"))?;
        if uuid::Uuid::parse_str(&payload.room_id).is_err() {
            return Err(error(
                id,
                ErrorCode::InvalidPayload,
                "Invalid room key request",
            ));
        }
        let owner = parse_address(&payload.owner)
            .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room owner"))?;
        if Some(owner) != member.and_then(parse_address) {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "Room owner must match the authenticated wallet",
            ));
        }
        let current = {
            let state = self.view.read().map_err(|_| unavailable(id))?;
            state.room(&payload.room_id).map(|room| {
                (
                    room.name.clone(),
                    room.key_preparation.is_some(),
                    room.key_publication.clone(),
                )
            })
        };
        let context = match (current, payload.remove_member) {
            (None, None) => self
                .room_keys(id)?
                .initial_context(owner, payload.room_id)
                .await
                .map_err(|_| unavailable(id))?,
            (Some((name, pending, publication)), Some(removed)) if name == normalized_name => {
                if pending {
                    return Err(error(
                        id,
                        ErrorCode::MessageConflict,
                        "Room key rotation is already pending",
                    ));
                }
                let publication = publication.as_ref().ok_or_else(|| {
                    error(id, ErrorCode::InvalidPayload, "Room key is not active")
                })?;
                let previous = SignedRoomKeyPolicyV1::decode_canonical(
                    &bounded_hex(&publication.signed_policy, 96 * 1024)
                        .ok_or_else(|| unavailable(id))?,
                )
                .map_err(|_| unavailable(id))?;
                previous.verify_owner().map_err(|_| unavailable(id))?;
                if previous.policy.identity.owner != owner {
                    return Err(error(
                        id,
                        ErrorCode::AccessDenied,
                        "Room owner must match the authenticated wallet",
                    ));
                }
                let removed = parse_address(&removed).ok_or_else(|| {
                    error(id, ErrorCode::InvalidPayload, "Invalid member to remove")
                })?;
                self.room_keys(id)?
                    .successor_context(&previous, removed)
                    .await
                    .map_err(|_| {
                        error(
                            id,
                            ErrorCode::InvalidPayload,
                            "Invalid room key rotation request",
                        )
                    })?
            }
            _ => {
                return Err(error(
                    id,
                    ErrorCode::InvalidPayload,
                    "Room key request does not match current room state",
                ));
            }
        };
        Ok(Frame::ok(id, context))
    }

    #[cfg(feature = "room-key-demo")]
    async fn attest_room_key_setup(
        &self,
        frame: &Frame,
        member: Option<&str>,
    ) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let payload: RoomKeyBytesPayload = parse(frame)?;
        let bytes = bounded_hex(&payload.request, 32 * 1024)
            .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid signed room setup"))?;
        let setup = SignedRoomSetupV1::decode_canonical(&bytes)
            .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid signed room setup"))?;
        if Some(setup.request.intent.identity.owner) != member.and_then(parse_address) {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "Room owner must match the authenticated wallet",
            ));
        }
        let responses = self
            .room_keys(id)?
            .attest_setup(&bytes)
            .await
            .map_err(|_| unavailable(id))?;
        Ok(Frame::ok(id, serde_json::json!({"responses": responses})))
    }

    #[cfg(feature = "room-key-demo")]
    async fn activate_room_key(
        &self,
        frame: &Frame,
        agent_id: &str,
        member: Option<&str>,
        broker: &Broker,
        store: &Store,
        reconnect: &ReconnectManager,
    ) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let payload: ActivateRoomKeyPayload = parse(frame)?;
        let room_owner = member
            .and_then(parse_address)
            .ok_or_else(|| error(id, ErrorCode::AccessDenied, "Invalid member principal"))?;
        let signed_policy = SignedRoomKeyPolicyV1::decode_canonical(
            &bounded_hex(&payload.preparation.signed_policy, 96 * 1024)
                .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?,
        )
        .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?;
        signed_policy
            .verify_owner()
            .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?;
        let previous_policy = payload
            .preparation
            .previous_policy
            .as_deref()
            .map(|encoded| -> Result<SignedRoomKeyPolicyV1, Frame> {
                let bytes = bounded_hex(encoded, 96 * 1024).ok_or_else(|| {
                    error(id, ErrorCode::InvalidPayload, "Invalid room activation")
                })?;
                let policy = SignedRoomKeyPolicyV1::decode_canonical(&bytes)
                    .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?;
                policy
                    .verify_owner()
                    .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?;
                Ok(policy)
            })
            .transpose()?;
        if signed_policy.policy.identity.owner != room_owner {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "Room owner must match the authenticated wallet",
            ));
        }
        let normalized_name = crate::store::normalize_room_name(&payload.name)
            .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room activation"))?;
        if uuid::Uuid::parse_str(&payload.room_id).is_err() {
            return Err(error(
                id,
                ErrorCode::InvalidPayload,
                "Invalid room activation",
            ));
        }
        let limits = TierLimits::for_tier(
            &store
                .get_key_tier(&self.api_key)
                .unwrap_or_else(|_| "free".into()),
        );
        let input = crate::hosted_bootstrap::BrowserInitialRoom {
            room_id: payload.room_id.clone(),
            name: payload.name,
            created_by: agent_id.into(),
            room_owner,
            preparation: payload.preparation,
        };
        let wallet_limits = self.room_keys(id)?.wallet_room_limits();
        // The control volume has one mutable root per owner. Keep complete
        // room-key ceremonies ordered while leaving the general owner writer
        // available to existing rooms' message appends.
        let _activation = self.room_key_activation.lock().await;
        // Exact committed retries are local: reconcile live state and replay
        // the event without contacting CBSS again.
        let transition = {
            let writer = self.writer.lock().await;
            let transition = {
                let state = writer.state().map_err(|_| unavailable(id))?;
                classify_room_key_activation(
                    &state,
                    &input,
                    &normalized_name,
                    room_owner,
                    limits.max_rooms,
                    wallet_limits,
                    id,
                )?
            };
            if transition == RoomKeyActivationTransition::Committed {
                let room = {
                    let state = writer.state().map_err(|_| unavailable(id))?;
                    room_summary(state.room(&input.room_id).ok_or_else(|| unavailable(id))?)
                };
                reconcile_room_key_commit(
                    broker,
                    reconnect,
                    &self.api_key,
                    &room,
                    previous_policy.as_ref(),
                    &signed_policy,
                );
                return Ok(Frame::ok(id, serde_json::to_value(room).unwrap()));
            }
            transition
        };
        self.room_keys(id)?.preflight(&input).await.map_err(|_| {
            error(
                id,
                ErrorCode::MessageConflict,
                "Room key preparation conflicts with current control state",
            )
        })?;
        // Reserve only the local log records under the owner writer. CBSS
        // publication and finality remain outside this lock.
        {
            let mut writer = self.writer.lock().await;
            self.room_keys(id)?
                .stage(
                    &mut writer,
                    &input,
                    transition == RoomKeyActivationTransition::Stage,
                )
                .await
                .map_err(|_| unavailable(id))?;
        }
        // Publish + finalize with the owner-write lock released.
        let prepared = self
            .room_keys(id)?
            .finalize(input)
            .await
            .map_err(|_| unavailable(id))?;
        // Commit and reconcile membership under the same writer lock. A send
        // that was authorized against the predecessor cannot pass the second
        // membership check below after this lock is released.
        let room = {
            let mut writer = self.writer.lock().await;
            self.room_keys(id)?
                .commit(&mut writer, &prepared)
                .await
                .map_err(|_| unavailable(id))?;
            let room = {
                let state = writer.state().map_err(|_| unavailable(id))?;
                room_summary(
                    state
                        .room(&payload.room_id)
                        .ok_or_else(|| unavailable(id))?,
                )
            };
            reconcile_room_key_commit(
                broker,
                reconnect,
                &self.api_key,
                &room,
                previous_policy.as_ref(),
                &signed_policy,
            );
            room
        };
        Ok(Frame::ok(id, serde_json::to_value(room).unwrap()))
    }

    #[cfg(feature = "room-key-demo")]
    fn room_key_context(&self, frame: &Frame, principal: Option<&str>) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let payload: RoomKeyContextPayload = parse(frame)?;
        let member = parse_address(&payload.member)
            .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room member"))?;
        if Some(member) != principal.and_then(parse_address) {
            return Err(error(
                id,
                ErrorCode::AccessDenied,
                "Room member must match the authenticated wallet",
            ));
        }
        let state = self.view.read().map_err(|_| unavailable(id))?;
        let room = state
            .room(&payload.room_id)
            .ok_or_else(|| error(id, ErrorCode::RoomNotFound, "Room not found"))?;
        let publication = room
            .key_publication
            .as_ref()
            .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Room key is not active"))?;
        let policy = SignedRoomKeyPolicyV1::decode_canonical(
            &bounded_hex(&publication.signed_policy, 96 * 1024).ok_or_else(|| unavailable(id))?,
        )
        .map_err(|_| unavailable(id))?;
        let grants = member_grants(&policy, &publication.grants, member)
            .ok_or_else(|| error(id, ErrorCode::AccessDenied, "Wallet is not a room member"))?;
        let member_contexts =
            member_key_contexts(grants, &room.key_custodies).ok_or_else(|| unavailable(id))?;
        let mut contexts = Vec::with_capacity(member_contexts.len());
        let mut active_input = None;
        for member_context in member_contexts {
            let context = self
                .room_keys(id)?
                .open_context(
                    &policy,
                    &member_context.grant,
                    &member_context.custody_bytes,
                )
                .map_err(|_| unavailable(id))?;
            let context_object = context.as_object().ok_or_else(|| unavailable(id))?;
            let input = context_object
                .get("input")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| unavailable(id))?;
            let encoded_grant = input
                .get("grant")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| unavailable(id))?;
            if member_context.key_epoch == policy.policy.active_key_epoch {
                active_input = Some(serde_json::Value::Object(input.clone()));
            }
            contexts.push(serde_json::json!({
                "grant": encoded_grant,
                "custody": member_context.custody,
                "key_epoch": member_context.key_epoch.to_string(),
            }));
        }
        let context = serde_json::json!({
            "input": active_input.ok_or_else(|| unavailable(id))?,
            "contexts": contexts,
            "key_epoch": policy.policy.active_key_epoch.to_string(),
            "policy_owner": format!(
                "0x{}",
                hex::encode(policy.policy.identity.owner.as_bytes())
            ),
            "policy_members": policy
                .policy
                .members
                .iter()
                .map(|address| format!("0x{}", hex::encode(address.as_bytes())))
                .collect::<Vec<_>>(),
        });
        if serde_json::to_vec(&context)
            .map_err(|_| unavailable(id))?
            .len()
            > crate::server::MAX_FRAME_BYTES.saturating_sub(1024)
        {
            return Err(unavailable(id));
        }
        Ok(Frame::ok(id, context))
    }

    #[cfg(feature = "room-key-demo")]
    async fn relay_room_key_open(&self, frame: &Frame) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let payload: RoomKeyBytesPayload = parse(frame)?;
        let bytes = bounded_hex(&payload.request, 64 * 1024)
            .ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room-key request"))?;
        let responses = self
            .room_keys(id)?
            .relay_open(&bytes)
            .await
            .map_err(|_| unavailable(id))?;
        Ok(Frame::ok(id, serde_json::json!({"responses": responses})))
    }

    async fn create(
        &self,
        frame: &Frame,
        agent_id: &str,
        broker: &Broker,
        store: &Store,
        reconnect: &ReconnectManager,
    ) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let p: CreateRoomPayload = parse(frame)?;
        if p.public || !p.encrypted || p.parent_id.is_some() || p.description.is_some() {
            return Err(error(id, ErrorCode::UnsupportedProtocol, "Hosted creation currently supports private encrypted rooms without parent or description"));
        }
        let room_id = p
            .room_id
            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
            .ok_or_else(|| {
                error(
                    id,
                    ErrorCode::InvalidPayload,
                    "Retain a prepared UUID room_id for hosted creation retries",
                )
            })?;
        let name = crate::store::normalize_room_name(&p.name)
            .map_err(|_| error(id, ErrorCode::InvalidPayload, "Invalid room name"))?;
        let mut writer = self.writer.lock().await;
        let limits = TierLimits::for_tier(
            &store
                .get_key_tier(&self.api_key)
                .unwrap_or_else(|_| "free".into()),
        );
        {
            let state = writer.state().map_err(|_| unavailable(id))?;
            if state.room(&room_id).is_none() && state.rooms().len() as u64 >= limits.max_rooms {
                return Err(error(id, ErrorCode::RateLimitRooms, "Room limit exceeded"));
            }
        }
        let command = Command {
            owner_id: self.owner_id.clone(),
            command_id: format!("create:{room_id}"),
            timestamp: chrono::Utc::now(),
            body: CommandBody::CreateRoom {
                room_id: room_id.clone(),
                lane_id: 0,
                name,
                created_by: agent_id.into(),
            },
        };
        let batch = writer
            .submit(vec![command])
            .await
            .map_err(|_| unavailable(id))?;
        rejected(id, &batch.outcomes[0])?;
        let room = self.require_room(id, &room_id)?;
        if !batch.applied.is_empty() {
            let event = Frame::event(FrameType::RoomCreated, serde_json::to_value(&room).unwrap());
            let buffered = reconnect.buffer_visible_room_event(
                "private",
                Some(&self.api_key),
                &HashSet::new(),
                false,
                &event,
            );
            let recipients: Vec<_> = broker
                .agents
                .iter()
                .filter(|agent| agent.api_key == self.api_key && !buffered.contains(agent.key()))
                .map(|agent| agent.key().clone())
                .collect();
            for agent in recipients {
                broker.send_to_agent(&agent, event.clone());
            }
        }
        Ok(Frame::ok(id, serde_json::to_value(room).unwrap()))
    }

    async fn send(
        &self,
        frame: &Frame,
        agent_id: &str,
        agent_name: &str,
        broker: &Broker,
        store: &Store,
        rates: &RateLimiter,
    ) -> Result<Frame, Frame> {
        let id = frame.id.as_deref();
        let p: SendMessagePayload = parse(frame)?;
        let key_epoch = p
            .key_epoch
            .as_deref()
            .map(|text| {
                let epoch = text
                    .parse::<u64>()
                    .ok()
                    .filter(|epoch| epoch.to_string() == text);
                epoch.ok_or_else(|| error(id, ErrorCode::InvalidPayload, "Invalid room key epoch"))
            })
            .transpose()?;
        self.require_room(id, &p.room_id)?;
        if !broker.is_agent_in_room(agent_id, &p.room_id) {
            return Err(error(id, ErrorCode::NotInRoom, "Not in this room"));
        }
        if !crypto::is_ciphertext(&p.content) {
            return Err(error(
                id,
                ErrorCode::PlaintextInEncryptedRoom,
                "Encrypt the message before sending",
            ));
        }
        let message_id = p
            .message_id
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .ok_or_else(|| {
                error(
                    id,
                    ErrorCode::InvalidPayload,
                    "Retain a prepared message_id and ciphertext for hosted retries",
                )
            })?;
        let command = Command {
            owner_id: self.owner_id.clone(),
            command_id: message_id,
            timestamp: chrono::Utc::now(),
            body: CommandBody::AppendMessage {
                room_id: p.room_id.clone(),
                key_epoch,
                agent_id: agent_id.into(),
                agent_name: agent_name.into(),
                ciphertext: p.content,
                reply_to: p.reply_to,
                metadata: p.metadata,
                mentions: p.mentions,
            },
        };
        let mut writer = self.writer.lock().await;
        if !broker.is_agent_in_room(agent_id, &p.room_id) {
            return Err(error(id, ErrorCode::NotInRoom, "Not in this room"));
        }
        let seen = writer
            .state()
            .map_err(|_| unavailable(id))?
            .receipt(&command)
            .map_err(|_| unavailable(id))?
            .is_some();
        if !seen {
            let limits = TierLimits::for_tier(
                &store
                    .get_key_tier(&self.api_key)
                    .unwrap_or_else(|_| "free".into()),
            );
            if let CommandBody::AppendMessage { ciphertext, .. } = &command.body {
                if ciphertext.len() > limits.max_message_bytes {
                    return Err(error(
                        id,
                        ErrorCode::MessageTooLarge,
                        "Message exceeds the owner's byte limit",
                    ));
                }
            }
            if !rates.check_message_rate(&self.api_key, &limits) {
                return Err(error(
                    id,
                    ErrorCode::RateLimitMessages,
                    "Message rate limit exceeded",
                ));
            }
        }
        let batch = writer
            .submit(vec![command])
            .await
            .map_err(|_| unavailable(id))?;
        rejected(id, &batch.outcomes[0])?;
        let Outcome::MessageAppended { sequence, .. } = &batch.outcomes[0] else {
            return Err(unavailable(id));
        };
        let stored = {
            let state = self.view.read().map_err(|_| unavailable(id))?;
            state
                .room(&p.room_id)
                .and_then(|room| room.messages.get((*sequence - 1) as usize))
                .cloned()
                .ok_or_else(|| unavailable(id))?
        };
        if !batch.applied.is_empty() {
            rates.increment_message(&self.api_key);
            if let Some(mut agent) = broker.agents.get_mut(agent_id) {
                agent.info.last_active = Some(stored.message.timestamp);
            }
            let event = Frame::event(
                FrameType::MessageReceived,
                serde_json::to_value(&stored.message).unwrap(),
            );
            broker.broadcast_to_room(&p.room_id, agent_id, &event);
            broker.send_mentions(&stored.mentions, &stored.message, &p.room_id);
            broker.advance_turn_from(&p.room_id, agent_id);
            crate::handler::broadcast_turn_changed(broker, &p.room_id, "message_sent");
        }
        Ok(Frame::ok(id, serde_json::to_value(stored.message).unwrap()))
    }
}
