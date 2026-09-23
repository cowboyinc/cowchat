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
use commonware_codec::Decode;
#[cfg(feature = "room-key-demo")]
use cowboy_protocol_codec::{
    room_policy::SignedRoomKeyPolicyV1, room_release::SignedRoomKeyGrantV1,
    room_setup::SignedRoomSetupV1, room_transport::RoomKeyCallV1, Address,
};

#[cfg(feature = "room-key-demo")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRoomKeyPayload {
    room_id: String,
    name: String,
    owner: String,
}

#[cfg(all(test, feature = "room-key-demo"))]
mod member_policy_tests {
    use super::*;
    use commonware_codec::Encode;
    use cowboy_protocol_codec::{
        room_policy::{
            RoomCustodyCommitmentV1, RoomKeyPolicyV1, RoomPolicyIdentityV1, SignedRoomKeyPolicyV1,
        },
        EthSignature,
    };
    use k256::ecdsa::SigningKey;

    #[test]
    fn hosted_member_access_comes_only_from_the_signed_roster() {
        let owner = SigningKey::from_bytes((&[0x11; 32]).into()).unwrap();
        let member = SigningKey::from_bytes((&[0x22; 32]).into()).unwrap();
        let member_address = Address::from_verifying_key(member.verifying_key());
        let mut members = vec![
            Address::from_verifying_key(owner.verifying_key()),
            member_address,
        ];
        members.sort_unstable();
        let policy = RoomKeyPolicyV1 {
            identity: RoomPolicyIdentityV1 {
                chain_id: 1,
                chain_instance_id: [1; 32],
                service_id: [2; 32],
                owner: Address::from_verifying_key(owner.verifying_key()),
                room_id: "11111111-1111-4111-8111-111111111111".into(),
            },
            policy_epoch: 0,
            active_key_epoch: 0,
            members,
            keys: vec![RoomCustodyCommitmentV1 {
                key_epoch: 0,
                scope_hash: [3; 32],
                ciphertext_hash: [4; 32],
            }],
        };
        let signed = SignedRoomKeyPolicyV1 {
            owner_signature: EthSignature::sign(&owner, &policy.signing_hash()),
            policy,
        };
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
        if uuid::Uuid::parse_str(&payload.room_id).is_err()
            || crate::store::normalize_room_name(&payload.name).is_err()
            || self
                .view
                .read()
                .map_err(|_| unavailable(id))?
                .room(&payload.room_id)
                .is_some()
        {
            return Err(error(
                id,
                ErrorCode::InvalidPayload,
                "Invalid new room key request",
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
        let context = self
            .room_keys(id)?
            .initial_context(owner, payload.room_id)
            .await
            .map_err(|_| unavailable(id))?;
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
        // Classify and (for a fresh room) reserve it under the owner-write lock,
        // then release the lock. The CBSS publication ceremony below must not be
        // run under this lock: its up-to-120s finality wait would otherwise
        // serialize every concurrent send for this owner.
        {
            let mut writer = self.writer.lock().await;
            let is_fresh = {
                let state = writer.state().map_err(|_| unavailable(id))?;
                match state.room(&input.room_id) {
                    Some(existing)
                        if existing.name == normalized_name
                            && existing.key_publication.as_deref() == Some(&input.preparation)
                            && existing.key_state.is_some() =>
                    {
                        // Already committed: idempotent success.
                        return Ok(Frame::ok(
                            id,
                            serde_json::to_value(room_summary(existing)).unwrap(),
                        ));
                    }
                    Some(existing)
                        if existing.name == normalized_name
                            && existing.key_preparation.as_deref() == Some(&input.preparation)
                            && existing.key_state.is_none() =>
                    {
                        // Prepared but never committed — a prior finalize failed
                        // or timed out. Resume the ceremony rather than leaving
                        // the room permanently wedged.
                        false
                    }
                    Some(_) => {
                        return Err(error(
                            id,
                            ErrorCode::MessageConflict,
                            "Room activation conflicts with committed state",
                        ));
                    }
                    None if state.rooms().len() as u64 >= limits.max_rooms => {
                        return Err(error(id, ErrorCode::RateLimitRooms, "Room limit exceeded"));
                    }
                    None => {
                        let (active, pending) = wallet_room_usage(&state, room_owner);
                        if active + pending >= wallet_limits.0 || pending >= wallet_limits.1 {
                            return Err(error(
                                id,
                                ErrorCode::RateLimitRooms,
                                "Wallet room limit exceeded",
                            ));
                        }
                        true
                    }
                }
            };
            if is_fresh {
                self.room_keys(id)?
                    .stage(&mut writer, &input, true)
                    .await
                    .map_err(|_| unavailable(id))?;
            }
        }
        // Publish + finalize with the owner-write lock released.
        let prepared = self
            .room_keys(id)?
            .finalize(input)
            .await
            .map_err(|_| unavailable(id))?;
        // Commit the finalized epoch back under the lock.
        {
            let mut writer = self.writer.lock().await;
            self.room_keys(id)?
                .commit(&mut writer, &prepared)
                .await
                .map_err(|_| unavailable(id))?;
        }
        let room = self.require_room(id, &payload.room_id)?;
        let event = Frame::event(FrameType::RoomCreated, serde_json::to_value(&room).unwrap());
        let buffered = reconnect.buffer_visible_room_event(
            "private",
            Some(&self.api_key),
            &HashSet::new(),
            false,
            &event,
        );
        let recipients = broker
            .agents
            .iter()
            .filter(|agent| agent.api_key == self.api_key && !buffered.contains(agent.key()))
            .map(|agent| agent.key().clone())
            .collect::<Vec<_>>();
        for agent in recipients {
            broker.send_to_agent(&agent, event.clone());
        }
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
        let grant = publication
            .grants
            .iter()
            .filter_map(|value| bounded_hex(value, 178))
            .filter_map(|bytes| SignedRoomKeyGrantV1::decode_cfg(bytes.as_slice(), &()).ok())
            .find(|grant| grant.grant.member == member)
            .ok_or_else(|| error(id, ErrorCode::AccessDenied, "Wallet is not a room member"))?;
        let custody = bounded_hex(&publication.custody, 157).ok_or_else(|| unavailable(id))?;
        let context = self
            .room_keys(id)?
            .open_context(&policy, &grant, &custody)
            .map_err(|_| unavailable(id))?;
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
