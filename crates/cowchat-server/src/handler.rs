use cowchat_core::*;
use std::collections::HashSet;
use std::sync::Arc;

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.replace('\n', "\\n")
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated.replace('\n', "\\n"))
    }
}

use crate::broker::Broker;

/// Whether `room_id` is an end-to-end encrypted room, checking ephemeral rooms
/// (in-memory) then persistent rooms (SQLite). Unknown rooms are not encrypted.
fn room_is_encrypted(
    room_id: &str,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    store: &Arc<Store>,
) -> bool {
    if let Some(room) = ephemeral_rooms.get(room_id) {
        return room.encrypted;
    }
    store
        .get_room(room_id)
        .ok()
        .flatten()
        .map(|r| r.encrypted)
        .unwrap_or(false)
}

/// Reject client-submitted plaintext `content` destined for an encrypted room.
/// The server can't read content, so it enforces the `cow1:` ciphertext marker
/// — this prevents a keyless/misconfigured agent from silently leaking plaintext
/// into a room whose whole point is that the operator can't read it.
fn reject_plaintext_in_encrypted_room(
    req_id: Option<&str>,
    content: &str,
    room_id: &str,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    store: &Arc<Store>,
) -> Option<Frame> {
    if room_is_encrypted(room_id, ephemeral_rooms, store)
        && !cowchat_core::crypto::is_ciphertext(content)
    {
        return Some(Frame::error(
            req_id,
            ErrorPayload::new(
                ErrorCode::PlaintextInEncryptedRoom,
                "This room is end-to-end encrypted; set COWCHAT_ROOM_KEY so the client encrypts before sending",
            ),
        ));
    }
    None
}

/// Reject message content over the tier's byte cap. A DoS guard so a single
/// send/thinking can't blow the server's memory or fill the disk.
fn reject_oversized(req_id: Option<&str>, content: &str, limits: &TierLimits) -> Option<Frame> {
    if content.len() > limits.max_message_bytes {
        return Some(Frame::error(
            req_id,
            ErrorPayload::new(
                ErrorCode::MessageTooLarge,
                format!(
                    "Message content is {} bytes; the limit is {} bytes",
                    content.len(),
                    limits.max_message_bytes
                ),
            ),
        ));
    }
    None
}

/// Authorize a room-scoped operation consistently. Public and system rooms are
/// readable by every authenticated key; private rooms are visible only to the
/// owning key. Membership requirements are enforced separately by operations
/// that mutate room coordination state.
fn authorize_room(
    req_id: Option<&str>,
    room_id: &str,
    agent_api_key: &str,
    no_auth: bool,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
) -> Result<Room, Frame> {
    let room = ephemeral_rooms
        .get(room_id)
        .map(|room| room.clone())
        .or_else(|| store.get_room(room_id).ok().flatten())
        .ok_or_else(|| {
            Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::RoomNotFound, "Room not found"),
            )
        })?;
    let allowed =
        no_auth || room.visibility == "public" || room.owner_key.as_deref() == Some(agent_api_key);
    if !allowed {
        return Err(Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::AccessDenied, "This room is private"),
        ));
    }
    Ok(room)
}
use crate::rate_limit::{RateLimiter, TierLimits};
use crate::store::Store;
use crate::tasks::TaskManager;
use crate::voting::VoteManager;

/// Broadcast a `turn_changed` event to every member of `room_id`.
///
/// `reason` is one of `"joined"`, `"left"`, `"disconnected"`, `"message_sent"`.
/// Reads the current holder and member order from the broker.
pub(crate) fn broadcast_turn_changed(broker: &Arc<Broker>, room_id: &str, reason: &str) {
    let payload = TurnChangedPayload {
        room_id: room_id.to_string(),
        current_turn_holder: broker.turn_holder(room_id),
        turn_order: broker.get_room_members(room_id),
        reason: reason.to_string(),
    };
    let frame = Frame::event(
        FrameType::TurnChanged,
        serde_json::to_value(&payload).unwrap_or_default(),
    );
    broker.broadcast_to_room_all(room_id, &frame);
}

/// Processes a single frame from a registered agent and returns a response frame.
/// Side effects (broadcast, persistence) happen inline.
pub async fn handle_frame(
    frame: Frame,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    vote_mgr: &Arc<VoteManager>,
    agent_api_key: &str,
    rate_limiter: &Arc<RateLimiter>,
    no_auth: bool,
    task_mgr: &Arc<TaskManager>,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let req_id = frame.id.as_deref();

    match frame.frame_type {
        FrameType::Ping => Frame::pong(req_id),

        FrameType::CreateRoom => {
            handle_create_room(
                req_id,
                frame.payload,
                agent_id,
                broker,
                store,
                ephemeral_rooms,
                agent_api_key,
                rate_limiter,
                no_auth,
            )
            .await
        }
        FrameType::JoinRoom => {
            handle_join_room(
                req_id,
                frame.payload,
                agent_id,
                agent_name,
                broker,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }
        FrameType::LeaveRoom => {
            handle_leave_room(
                req_id,
                frame.payload,
                agent_id,
                broker,
                ephemeral_rooms,
                no_auth,
            )
            .await
        }
        FrameType::SendMessage => {
            handle_send_message(
                req_id,
                frame.payload,
                agent_id,
                agent_name,
                broker,
                store,
                ephemeral_rooms,
                agent_api_key,
                rate_limiter,
                no_auth,
                webhook_mgr,
            )
            .await
        }
        FrameType::GetHistory => {
            handle_get_history(
                req_id,
                frame.payload,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }
        FrameType::ListRooms => {
            handle_list_rooms(
                req_id,
                frame.payload,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
                broker,
            )
            .await
        }
        FrameType::ListAgents => {
            handle_list_agents(
                req_id,
                frame.payload,
                broker,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }
        FrameType::RoomInfo => {
            handle_room_info(
                req_id,
                frame.payload,
                store,
                broker,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }
        FrameType::RoomTip => {
            handle_room_tip(
                req_id,
                frame.payload,
                store,
                broker,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }

        // Voting
        FrameType::CreateVote => {
            handle_create_vote(
                req_id,
                frame.payload,
                agent_id,
                broker,
                store,
                ephemeral_rooms,
                vote_mgr,
            )
            .await
        }
        FrameType::CastVote => {
            handle_cast_vote(
                req_id,
                frame.payload,
                agent_id,
                agent_name,
                broker,
                store,
                vote_mgr,
            )
            .await
        }
        FrameType::GetVoteStatus => {
            handle_get_vote_status(
                req_id,
                frame.payload,
                vote_mgr,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }
        FrameType::ListVotes => {
            handle_list_votes(
                req_id,
                frame.payload,
                vote_mgr,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }

        // Elections
        FrameType::ElectLeader => {
            handle_elect_leader(req_id, frame.payload, agent_id, broker, vote_mgr).await
        }
        FrameType::DeclineElection => {
            handle_decline_election(req_id, frame.payload, agent_id, broker, vote_mgr).await
        }
        FrameType::Decision => {
            handle_decision(
                req_id,
                frame.payload,
                agent_id,
                agent_name,
                broker,
                store,
                ephemeral_rooms,
                vote_mgr,
            )
            .await
        }

        // Presence
        FrameType::SetTyping => {
            handle_set_typing(req_id, frame.payload, agent_id, agent_name, broker).await
        }
        FrameType::SetPresence => {
            handle_set_presence(req_id, frame.payload, agent_id, agent_name, broker).await
        }

        // Thinking pulse
        FrameType::Thinking => {
            handle_thinking(
                req_id,
                frame.payload,
                agent_id,
                agent_name,
                broker,
                store,
                ephemeral_rooms,
                agent_api_key,
                rate_limiter,
                no_auth,
                webhook_mgr,
            )
            .await
        }

        // Webhook subscriptions
        FrameType::Subscribe => {
            handle_subscribe(
                req_id,
                frame.payload,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
                webhook_mgr,
            )
            .await
        }
        FrameType::Unsubscribe => {
            handle_unsubscribe(req_id, frame.payload, store, agent_api_key).await
        }
        FrameType::ListSubscriptions => {
            handle_list_subscriptions(req_id, frame.payload, store, agent_api_key).await
        }
        FrameType::EnableSubscription => {
            handle_enable_subscription(req_id, frame.payload, store, agent_api_key, webhook_mgr)
                .await
        }

        // Tasks
        FrameType::AssignTask => {
            handle_assign_task(req_id, frame.payload, agent_id, broker, task_mgr).await
        }
        FrameType::UpdateTask => {
            handle_update_task(req_id, frame.payload, agent_id, broker, task_mgr).await
        }
        FrameType::ListTasks => {
            handle_list_tasks(
                req_id,
                frame.payload,
                task_mgr,
                store,
                ephemeral_rooms,
                agent_api_key,
                no_auth,
            )
            .await
        }

        _ => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InvalidPayload, "Unknown command"),
        ),
    }
}

async fn handle_create_room(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    rate_limiter: &Arc<RateLimiter>,
    no_auth: bool,
) -> Frame {
    let p: CreateRoomPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Check room limit (skip in no_auth mode)
    if !no_auth && !agent_api_key.is_empty() {
        let tier = store
            .get_key_tier(agent_api_key)
            .unwrap_or_else(|_| "free".to_string());
        let limits = TierLimits::for_tier(&tier);
        if !rate_limiter.check_room_limit(agent_api_key, &limits) {
            return Frame::error(
                req_id,
                ErrorPayload::new(
                    ErrorCode::RateLimitRooms,
                    "Room limit exceeded for this API key",
                ),
            );
        }
    }

    let room_id = uuid::Uuid::new_v4().to_string();
    let visibility = if p.public { "public" } else { "private" };

    if p.ephemeral {
        let room = Room {
            room_id: room_id.clone(),
            name: p.name.clone(),
            description: p.description,
            parent_id: p.parent_id,
            ephemeral: true,
            created_at: chrono::Utc::now(),
            created_by: Some(agent_id.to_string()),
            visibility: visibility.to_string(),
            owner_key: if agent_api_key.is_empty() {
                None
            } else {
                Some(agent_api_key.to_string())
            },
            last_activity: None,
            member_count: None,
            encrypted: p.encrypted,
        };
        ephemeral_rooms.insert(room_id.clone(), room.clone());

        // Track in rate limiter
        if !agent_api_key.is_empty() {
            rate_limiter.add_room(agent_api_key);
        }

        // Private room metadata is visible only to connections using the owner key.
        let event = Frame::event(FrameType::RoomCreated, serde_json::to_value(&room).unwrap());
        for entry in broker.agents.iter() {
            if no_auth || room.visibility == "public" || entry.value().api_key == agent_api_key {
                broker.send_to_agent(entry.key(), event.clone());
            }
        }

        Frame::ok(req_id, serde_json::to_value(&room).unwrap())
    } else {
        // Validate parent exists if specified
        if let Some(ref pid) = p.parent_id {
            if let Err(error) =
                authorize_room(req_id, pid, agent_api_key, no_auth, store, ephemeral_rooms)
            {
                return error;
            }
        }

        let owner_key = if agent_api_key.is_empty() {
            None
        } else {
            Some(agent_api_key)
        };

        match store.create_room_with_visibility(
            &room_id,
            &p.name,
            p.description.as_deref(),
            p.parent_id.as_deref(),
            Some(agent_id),
            visibility,
            owner_key,
            p.encrypted,
        ) {
            Ok(room) => {
                // Track in rate limiter
                if !agent_api_key.is_empty() {
                    rate_limiter.add_room(agent_api_key);
                }

                let event =
                    Frame::event(FrameType::RoomCreated, serde_json::to_value(&room).unwrap());
                for entry in broker.agents.iter() {
                    if no_auth
                        || room.visibility == "public"
                        || entry.value().api_key == agent_api_key
                    {
                        broker.send_to_agent(entry.key(), event.clone());
                    }
                }
                Frame::ok(req_id, serde_json::to_value(&room).unwrap())
            }
            Err(crate::store::StoreError::RoomNameTaken(name)) => Frame::error(
                req_id,
                ErrorPayload::new(
                    ErrorCode::RoomNameTaken,
                    format!("Room name '{}' already taken", name),
                ),
            ),
            Err(e) => Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            ),
        }
    }
}

async fn handle_join_room(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: JoinRoomPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Check room exists and get visibility info
    let (room_exists, room_visibility, room_owner_key) = {
        if let Some(room) = ephemeral_rooms.get(&p.room_id) {
            (true, room.visibility.clone(), room.owner_key.clone())
        } else if let Ok(Some(room)) = store.get_room(&p.room_id) {
            (true, room.visibility.clone(), room.owner_key.clone())
        } else {
            (false, String::new(), None)
        }
    };

    if !room_exists {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::RoomNotFound, "Room not found"),
        );
    }

    // Check visibility: private rooms require matching API key
    if !no_auth && room_visibility == "private" {
        let can_access = match &room_owner_key {
            Some(owner) => owner == agent_api_key,
            None => true, // System rooms (no owner) are accessible to all
        };
        if !can_access {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::AccessDenied, "This room is private"),
            );
        }
    }

    if broker.is_agent_in_room(agent_id, &p.room_id) {
        // Idempotent: joining a room you're already in is a no-op success, not
        // an error. (A stable-identity agent that took over a connection keeps
        // its membership, and callers commonly join-before-send.)
        return Frame::ok(req_id, serde_json::json!({"room_id": p.room_id}));
    }

    let join_outcome = broker.join_room(agent_id, &p.room_id);

    // Track room in agent connection
    if let Some(mut agent) = broker.agents.get_mut(agent_id) {
        agent.rooms.insert(p.room_id.clone());
    }

    // Broadcast join to other room members
    let event = Frame::event(
        FrameType::AgentJoined,
        serde_json::json!({
            "room_id": p.room_id,
            "agent": {
                "agent_id": agent_id,
                "name": agent_name,
            }
        }),
    );
    broker.broadcast_to_room(&p.room_id, agent_id, &event);

    if join_outcome.new_holder.is_some() {
        broadcast_turn_changed(broker, &p.room_id, "joined");
    }

    Frame::ok(req_id, serde_json::json!({"room_id": p.room_id}))
}

async fn handle_leave_room(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    no_auth: bool,
) -> Frame {
    let p: LeaveRoomPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Not in this room"),
        );
    }

    // Broadcast leave before removing
    let event = Frame::event(
        FrameType::AgentLeft,
        serde_json::json!({
            "room_id": p.room_id,
            "agent_id": agent_id,
        }),
    );
    broker.broadcast_to_room(&p.room_id, agent_id, &event);

    let leave_outcome = broker.leave_room(agent_id, &p.room_id);

    // Track in agent connection
    if let Some(mut agent) = broker.agents.get_mut(agent_id) {
        agent.rooms.remove(&p.room_id);
    }

    // Notify remaining members of the turn change before we potentially destroy the room.
    if leave_outcome.holder_changed && !leave_outcome.now_empty {
        broadcast_turn_changed(broker, &p.room_id, "left");
    }

    // Destroy ephemeral room if empty
    if leave_outcome.now_empty {
        if let Some((_, room)) = ephemeral_rooms.remove(&p.room_id) {
            broker.remove_room(&p.room_id);
            broker.forget_ephemeral_seq(&p.room_id);
            let destroy_event = Frame::event(
                FrameType::RoomDestroyed,
                serde_json::json!({"room_id": p.room_id}),
            );
            for entry in broker.agents.iter() {
                if no_auth
                    || room.visibility == "public"
                    || entry.value().api_key.as_str() == room.owner_key.as_deref().unwrap_or("")
                {
                    broker.send_to_agent(entry.key(), destroy_event.clone());
                }
            }
            log::info!("Ephemeral room {} destroyed (empty)", p.room_id);
        }
    }

    Frame::ok(req_id, serde_json::json!({"room_id": p.room_id}))
}

async fn handle_send_message(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    rate_limiter: &Arc<RateLimiter>,
    no_auth: bool,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let p: SendMessagePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Not in this room"),
        );
    }

    if let Some(err) =
        reject_plaintext_in_encrypted_room(req_id, &p.content, &p.room_id, ephemeral_rooms, store)
    {
        return err;
    }

    // The turn token is advisory: we publish whose turn it is and advance it on
    // every send, but we do NOT reject sends from non-holders. This avoids the
    // deadlock where the holder is stuck in `wait` and nobody else can speak.

    // Check size + message rate limit (skip in no_auth mode)
    if !no_auth && !agent_api_key.is_empty() {
        let tier = store
            .get_key_tier(agent_api_key)
            .unwrap_or_else(|_| "free".to_string());
        let limits = TierLimits::for_tier(&tier);
        if let Some(err) = reject_oversized(req_id, &p.content, &limits) {
            return err;
        }
        if !rate_limiter.check_message_rate(agent_api_key, &limits) {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::RateLimitMessages, "Message rate limit exceeded"),
            );
        }
    }

    let message_id = uuid::Uuid::new_v4().to_string();
    let is_ephemeral = ephemeral_rooms.contains_key(&p.room_id);

    let message = if is_ephemeral {
        // Ephemeral rooms: don't persist messages, but still assign a per-room seq.
        ChatMessage {
            message_id: message_id.clone(),
            room_id: p.room_id.clone(),
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            content: p.content,
            reply_to_message: p.reply_to,
            metadata: p.metadata,
            timestamp: chrono::Utc::now(),
            seq: broker.next_ephemeral_seq(&p.room_id),
        }
    } else {
        // Permanent rooms: persist to SQLite
        match store.insert_message(
            &message_id,
            &p.room_id,
            agent_id,
            agent_name,
            &p.content,
            p.reply_to.as_deref(),
            &p.metadata,
        ) {
            Ok(msg) => msg,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    };

    // Track in rate limiter
    if !agent_api_key.is_empty() {
        rate_limiter.increment_message(agent_api_key);
    }

    // Update last_active on the agent
    if let Some(mut agent) = broker.agents.get_mut(agent_id) {
        agent.info.last_active = Some(chrono::Utc::now());
    }

    log::debug!(
        "msg #{} room={} from={} ({}): {}",
        message.seq,
        message.room_id,
        message.agent_name,
        agent_id,
        truncate_for_log(&message.content, 500),
    );

    // Broadcast to room members (excluding sender)
    let event = Frame::event(
        FrameType::MessageReceived,
        serde_json::to_value(&message).unwrap(),
    );
    broker.broadcast_to_room(&p.room_id, agent_id, &event);

    // Handle @mentions
    if !p.mentions.is_empty() {
        log::debug!(
            "msg #{} room={} mentions={:?}",
            message.seq,
            message.room_id,
            p.mentions
        );
        broker.send_mentions(&p.mentions, &message, &p.room_id);
    }

    // Advance the turn token to the next member after the sender (join order),
    // then notify the room. "Whoever just spoke passes to the next."
    broker.advance_turn_from(&p.room_id, agent_id);
    broadcast_turn_changed(broker, &p.room_id, "message_sent");

    // Fire-and-forget: enqueue webhook deliveries to any matching subscriptions.
    // This spawns a separate task internally, so it doesn't block the send path.
    webhook_mgr.enqueue_for_message(&message);

    Frame::ok(req_id, serde_json::to_value(&message).unwrap())
}

#[allow(clippy::too_many_arguments)]
async fn handle_thinking(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    rate_limiter: &Arc<RateLimiter>,
    no_auth: bool,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let p: ThinkingPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Not in this room"),
        );
    }

    if let Some(err) =
        reject_plaintext_in_encrypted_room(req_id, &p.content, &p.room_id, ephemeral_rooms, store)
    {
        return err;
    }

    // Thinking shares the same size + rate-limit bucket as send_message — an agent
    // can't dodge limits by routing chat through `thinking`.
    if !no_auth && !agent_api_key.is_empty() {
        let tier = store
            .get_key_tier(agent_api_key)
            .unwrap_or_else(|_| "free".to_string());
        let limits = TierLimits::for_tier(&tier);
        if let Some(err) = reject_oversized(req_id, &p.content, &limits) {
            return err;
        }
        if !rate_limiter.check_message_rate(agent_api_key, &limits) {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::RateLimitMessages, "Message rate limit exceeded"),
            );
        }
    }

    let message_id = uuid::Uuid::new_v4().to_string();
    let is_ephemeral = ephemeral_rooms.contains_key(&p.room_id);
    let metadata = serde_json::json!({"type": "thinking"});

    let message = if is_ephemeral {
        ChatMessage {
            message_id: message_id.clone(),
            room_id: p.room_id.clone(),
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            content: p.content,
            reply_to_message: None,
            metadata,
            timestamp: chrono::Utc::now(),
            seq: broker.next_ephemeral_seq(&p.room_id),
        }
    } else {
        match store.insert_message(
            &message_id,
            &p.room_id,
            agent_id,
            agent_name,
            &p.content,
            None,
            &metadata,
        ) {
            Ok(msg) => msg,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    };

    if !agent_api_key.is_empty() {
        rate_limiter.increment_message(agent_api_key);
    }

    log::debug!(
        "thinking #{} room={} from={} ({}): {}",
        message.seq,
        message.room_id,
        message.agent_name,
        agent_id,
        truncate_for_log(&message.content, 500),
    );

    // Broadcast as a `thinking` event (NOT message_received), so `wait`-style
    // listeners that key on chat messages won't wake on every pulse. Token is
    // deliberately not advanced — thinking out loud doesn't pass the turn.
    let event = Frame::event(FrameType::Thinking, serde_json::to_value(&message).unwrap());
    broker.broadcast_to_room(&p.room_id, agent_id, &event);

    // Webhook subscriptions also see thinking pulses (unless they opted out
    // via `exclude_thinking`). The enqueue path filters per subscription.
    webhook_mgr.enqueue_for_message(&message);

    Frame::ok(req_id, serde_json::to_value(&message).unwrap())
}

async fn handle_get_history(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: GetHistoryPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if let Err(error) = authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    match store.get_history_filtered(
        &p.room_id,
        p.limit,
        p.before,
        p.since.as_deref(),
        p.since_seq,
    ) {
        Ok(messages) => Frame {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: req_id.map(String::from),
            frame_type: FrameType::HistoryResult,
            payload: serde_json::json!({
                "room_id": p.room_id,
                "messages": messages,
            }),
        },
        Err(e) => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
        ),
    }
}

async fn handle_room_tip(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    broker: &Arc<Broker>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: RoomTipPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if let Err(error) = authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    let seq = if ephemeral_rooms.contains_key(&p.room_id) {
        broker.ephemeral_room_tip(&p.room_id)
    } else {
        match store.room_tip(&p.room_id) {
            Ok(s) => s,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    };

    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: req_id.map(String::from),
        frame_type: FrameType::RoomTipResult,
        payload: serde_json::json!({
            "room_id": p.room_id,
            "seq": seq,
        }),
    }
}

async fn handle_list_rooms(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
    broker: &Arc<Broker>,
) -> Frame {
    let p: ListRoomsPayload =
        serde_json::from_value(payload).unwrap_or(ListRoomsPayload { parent_id: None });

    // In no_auth mode, show all rooms. In cloud mode, show public + owned by this key.
    let mut rooms = if no_auth {
        match store.list_rooms(p.parent_id.as_deref()) {
            Ok(r) => r,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    } else {
        let key = if agent_api_key.is_empty() {
            None
        } else {
            Some(agent_api_key)
        };
        match store.list_rooms_for_key(key, p.parent_id.as_deref()) {
            Ok(r) => r,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    };

    // Include ephemeral rooms (with visibility filtering in cloud mode)
    for entry in ephemeral_rooms.iter() {
        let room = entry.value();
        let matches_parent = match &p.parent_id {
            Some(pid) => room.parent_id.as_deref() == Some(pid.as_str()),
            None => true,
        };
        if !matches_parent {
            continue;
        }

        if no_auth {
            rooms.push(room.clone());
        } else {
            // In cloud mode, only show public ephemeral rooms or rooms owned by this key
            let visible =
                room.visibility == "public" || room.owner_key.as_deref() == Some(agent_api_key);
            if visible {
                rooms.push(room.clone());
            }
        }
    }

    // Enrich rooms with member counts and last activity
    for room in &mut rooms {
        let count = broker.get_room_members(&room.room_id).len();
        room.member_count = Some(count);
        // Get last message timestamp from store for permanent rooms
        if !room.ephemeral {
            if let Ok(msgs) = store.get_history(&room.room_id, 1, None) {
                room.last_activity = msgs.last().map(|m| m.timestamp);
            }
        }
    }

    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: req_id.map(String::from),
        frame_type: FrameType::RoomList,
        payload: serde_json::json!({"rooms": rooms}),
    }
}

async fn handle_list_agents(
    req_id: Option<&str>,
    payload: serde_json::Value,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: ListAgentsPayload =
        serde_json::from_value(payload).unwrap_or(ListAgentsPayload { room_id: None });

    if let Some(room_id) = &p.room_id {
        if let Err(error) = authorize_room(
            req_id,
            room_id,
            agent_api_key,
            no_auth,
            store,
            ephemeral_rooms,
        ) {
            return error;
        }
    }

    let agents: Vec<AgentInfo> = match &p.room_id {
        Some(room_id) => {
            let member_ids = broker.get_room_members(room_id);
            member_ids
                .iter()
                .filter_map(|id| broker.agents.get(id).map(|a| a.info.clone()))
                .collect()
        }
        None => broker
            .agents
            .iter()
            .filter(|agent| no_auth || agent.api_key == agent_api_key)
            .map(|agent| agent.info.clone())
            .collect(),
    };

    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: req_id.map(String::from),
        frame_type: FrameType::AgentList,
        payload: serde_json::json!({"agents": agents}),
    }
}

async fn handle_room_info(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    broker: &Arc<Broker>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: RoomInfoPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    let room = match authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        Ok(room) => room,
        Err(error) => return error,
    };

    let members = broker.get_room_members(&p.room_id);
    let agents: Vec<AgentInfo> = members
        .iter()
        .filter_map(|id| broker.agents.get(id).map(|a| a.info.clone()))
        .collect();

    // Get sub-rooms
    let mut sub_rooms = store.list_rooms(Some(&p.room_id)).unwrap_or_default();
    for entry in ephemeral_rooms.iter() {
        if entry.value().parent_id.as_deref() == Some(&p.room_id) {
            sub_rooms.push(entry.value().clone());
        }
    }
    sub_rooms.retain(|child| {
        no_auth || child.visibility == "public" || child.owner_key.as_deref() == Some(agent_api_key)
    });

    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: req_id.map(String::from),
        frame_type: FrameType::RoomInfoResult,
        payload: serde_json::json!({
            "room": room,
            "agents": agents,
            "sub_rooms": sub_rooms,
            "current_turn_holder": broker.turn_holder(&p.room_id),
            "turn_order": members,
        }),
    }
}

// --- Voting handlers ---

async fn handle_create_vote(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    vote_mgr: &Arc<VoteManager>,
) -> Frame {
    let p: CreateVotePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Must be in the room
    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to create a vote"),
        );
    }

    if p.options.len() < 2 {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InvalidPayload, "Need at least 2 options"),
        );
    }

    let vote_id = uuid::Uuid::new_v4().to_string();
    let eligible_agents = broker.get_room_members(&p.room_id);
    let eligible = eligible_agents.len();
    let is_ephemeral = ephemeral_rooms.contains_key(&p.room_id);

    // Persist to SQLite if not ephemeral
    if !is_ephemeral {
        let closes_at = p
            .duration_secs
            .map(|s| chrono::Utc::now() + chrono::Duration::seconds(s as i64));
        if let Err(e) = store.create_vote(
            &vote_id,
            &p.room_id,
            &p.title,
            p.description.as_deref(),
            &p.options,
            agent_id,
            closes_at,
            &eligible_agents,
        ) {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            );
        }
    }

    // Create in-memory tracking (manages deadline timer)
    let vote = vote_mgr.create_vote(
        vote_id.clone(),
        p.room_id.clone(),
        p.title.clone(),
        p.description.clone(),
        p.options.clone(),
        agent_id.to_string(),
        p.duration_secs,
        eligible_agents,
        is_ephemeral,
        broker.clone(),
        store.clone(),
    );

    let info = VoteInfo {
        vote_id: vote.vote_id.clone(),
        room_id: vote.room_id.clone(),
        title: vote.title.clone(),
        description: vote.description.clone(),
        options: vote.options.clone(),
        created_by: vote.created_by.clone(),
        created_at: vote.created_at,
        closes_at: vote.closes_at,
        status: VoteStatus::Open,
        votes_cast: 0,
        eligible_voters: eligible,
        tally: None,
    };

    // Broadcast VoteCreated to room
    let event = Frame::event(FrameType::VoteCreated, serde_json::to_value(&info).unwrap());
    broker.broadcast_to_room_all(&p.room_id, &event);

    Frame::ok(req_id, serde_json::to_value(&info).unwrap())
}

async fn handle_cast_vote(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    vote_mgr: &Arc<VoteManager>,
) -> Frame {
    let p: CastVotePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Check if vote is active (in-memory)
    let (is_ephemeral, option_count, room_id, eligible) =
        match vote_mgr.active_votes.get(&p.vote_id) {
            Some(vote) => (
                vote.is_ephemeral,
                Some(vote.options.len()),
                Some(vote.room_id.clone()),
                vote.eligible_agents.contains(agent_id),
            ),
            None => {
                let eligible = store
                    .get_vote_eligible_agents(&p.vote_id)
                    .map(|agents| agents.iter().any(|candidate| candidate == agent_id))
                    .unwrap_or(false);
                (
                    false,
                    None,
                    store
                        .get_vote_meta(&p.vote_id)
                        .ok()
                        .flatten()
                        .map(|vote| vote.room_id),
                    eligible,
                )
            }
        };

    let Some(room_id) = room_id else {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::VoteNotFound, "Vote not found"),
        );
    };
    if !broker.is_agent_in_room(agent_id, &room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to vote"),
        );
    }
    if !eligible {
        return Frame::error(
            req_id,
            ErrorPayload::new(
                ErrorCode::AccessDenied,
                "Agent was not eligible when vote opened",
            ),
        );
    }

    // Validate option index before persisting ballot.
    if let Some(count) = option_count {
        if p.option_index >= count {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidOption, "Invalid option index"),
            );
        }
    }

    // Persist ballot to SQLite if not ephemeral
    if !is_ephemeral {
        if let Err(e) = store.cast_vote(&p.vote_id, agent_id, agent_name, p.option_index) {
            let code = match e {
                crate::store::StoreError::VoteNotFound => ErrorCode::VoteNotFound,
                crate::store::StoreError::VoteClosed => ErrorCode::VoteClosed,
                crate::store::StoreError::AlreadyVoted => ErrorCode::AlreadyVoted,
                _ => ErrorCode::InternalError,
            };
            return Frame::error(req_id, ErrorPayload::new(code, e.to_string()));
        }
    }

    // Record in-memory (may trigger close + broadcast)
    match vote_mgr
        .cast_vote(
            &p.vote_id,
            agent_id,
            agent_name,
            p.option_index,
            broker,
            store,
        )
        .await
    {
        Ok((votes_cast, eligible)) => Frame::ok(
            req_id,
            serde_json::json!({
                "vote_id": p.vote_id,
                "votes_cast": votes_cast,
                "eligible_voters": eligible,
            }),
        ),
        Err(code) => Frame::error(
            req_id,
            ErrorPayload::new(code.clone(), format!("{:?}", code)),
        ),
    }
}

fn vote_status_from_str(status: &str) -> VoteStatus {
    if status.eq_ignore_ascii_case("closed") {
        VoteStatus::Closed
    } else {
        VoteStatus::Open
    }
}

fn build_vote_tally(options: &[String], ballots: &[(String, String, usize)]) -> Vec<VoteTally> {
    let mut tally_counts = vec![0usize; options.len()];
    for (_, _, option_index) in ballots {
        if *option_index < tally_counts.len() {
            tally_counts[*option_index] += 1;
        }
    }

    options
        .iter()
        .enumerate()
        .map(|(idx, option)| VoteTally {
            option_index: idx,
            option_text: option.clone(),
            count: tally_counts[idx],
        })
        .collect()
}

fn vote_info_from_meta(
    meta: &crate::store::VoteMeta,
    votes_cast: usize,
    tally: Option<Vec<VoteTally>>,
) -> VoteInfo {
    VoteInfo {
        vote_id: meta.vote_id.clone(),
        room_id: meta.room_id.clone(),
        title: meta.title.clone(),
        description: meta.description.clone(),
        options: meta.options.clone(),
        created_by: meta.created_by.clone(),
        created_at: meta.created_at,
        closes_at: meta.closes_at,
        status: vote_status_from_str(&meta.status),
        votes_cast,
        eligible_voters: meta.eligible_voters,
        tally,
    }
}

fn load_closed_vote_tally(
    store: &Arc<Store>,
    vote_id: &str,
    options: &[String],
) -> Result<Vec<VoteTally>, crate::store::StoreError> {
    let ballots = store.get_vote_ballots(vote_id)?;
    Ok(build_vote_tally(options, &ballots))
}

async fn handle_get_vote_status(
    req_id: Option<&str>,
    payload: serde_json::Value,
    vote_mgr: &Arc<VoteManager>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: GetVoteStatusPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Check in-memory active votes first.
    if let Some(vote) = vote_mgr.active_votes.get(&p.vote_id) {
        if let Err(error) = authorize_room(
            req_id,
            &vote.room_id,
            agent_api_key,
            no_auth,
            store,
            ephemeral_rooms,
        ) {
            return error;
        }
        let info = VoteInfo {
            vote_id: vote.vote_id.clone(),
            room_id: vote.room_id.clone(),
            title: vote.title.clone(),
            description: vote.description.clone(),
            options: vote.options.clone(),
            created_by: vote.created_by.clone(),
            created_at: vote.created_at,
            closes_at: vote.closes_at,
            status: VoteStatus::Open,
            votes_cast: vote.ballots.len(),
            eligible_voters: vote.eligible_voters,
            tally: None,
        };
        return Frame::ok(req_id, serde_json::to_value(&info).unwrap());
    }

    // Fallback to persisted vote metadata (supports closed vote status queries).
    let Some(meta) = (match store.get_vote_meta(&p.vote_id) {
        Ok(meta) => meta,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            )
        }
    }) else {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::VoteNotFound, "Vote not found"),
        );
    };

    if let Err(error) = authorize_room(
        req_id,
        &meta.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    let votes_cast = match store.get_vote_ballot_count(&meta.vote_id) {
        Ok(v) => v,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            )
        }
    };

    let tally = if meta.status.eq_ignore_ascii_case("closed") {
        match load_closed_vote_tally(store, &meta.vote_id, &meta.options) {
            Ok(tally) => Some(tally),
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        }
    } else {
        None
    };

    let info = vote_info_from_meta(&meta, votes_cast, tally);
    Frame::ok(req_id, serde_json::to_value(&info).unwrap())
}

async fn handle_list_votes(
    req_id: Option<&str>,
    payload: serde_json::Value,
    vote_mgr: &Arc<VoteManager>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: ListVotesPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if let Err(error) = authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    let mut seen_vote_ids: HashSet<String> = HashSet::new();
    let mut votes: Vec<VoteInfo> = Vec::new();

    for entry in vote_mgr.active_votes.iter() {
        let vote = entry.value();
        if vote.room_id != p.room_id {
            continue;
        }

        seen_vote_ids.insert(vote.vote_id.clone());
        votes.push(VoteInfo {
            vote_id: vote.vote_id.clone(),
            room_id: vote.room_id.clone(),
            title: vote.title.clone(),
            description: vote.description.clone(),
            options: vote.options.clone(),
            created_by: vote.created_by.clone(),
            created_at: vote.created_at,
            closes_at: vote.closes_at,
            status: VoteStatus::Open,
            votes_cast: vote.ballots.len(),
            eligible_voters: vote.eligible_voters,
            tally: None,
        });
    }

    let persisted_votes = match store.list_votes(&p.room_id, p.limit) {
        Ok(votes) => votes,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            )
        }
    };

    for meta in persisted_votes {
        if seen_vote_ids.contains(&meta.vote_id) {
            continue;
        }

        let votes_cast = match store.get_vote_ballot_count(&meta.vote_id) {
            Ok(v) => v,
            Err(e) => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                )
            }
        };

        let tally = if meta.status.eq_ignore_ascii_case("closed") {
            match load_closed_vote_tally(store, &meta.vote_id, &meta.options) {
                Ok(tally) => Some(tally),
                Err(e) => {
                    return Frame::error(
                        req_id,
                        ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
                    )
                }
            }
        } else {
            None
        };

        votes.push(vote_info_from_meta(&meta, votes_cast, tally));
    }

    votes.sort_by_key(|vote| std::cmp::Reverse(vote.created_at));
    votes.truncate(p.limit as usize);

    Frame::ok(
        req_id,
        serde_json::json!({
            "room_id": p.room_id,
            "votes": votes,
        }),
    )
}

// --- Election handlers ---

async fn handle_elect_leader(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    vote_mgr: &Arc<VoteManager>,
) -> Frame {
    let p: ElectLeaderPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to start election"),
        );
    }

    let candidates = broker.get_room_members(&p.room_id);
    if candidates.is_empty() {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InvalidPayload, "No candidates in room"),
        );
    }

    match vote_mgr.start_election(&p.room_id, candidates.clone(), agent_id, broker.clone()) {
        Ok(()) => {
            // Broadcast ElectionStarted to room
            let event = Frame::event(
                FrameType::ElectionStarted,
                serde_json::json!({
                    "room_id": p.room_id,
                    "candidates": candidates,
                    "started_by": agent_id,
                    "opt_out_seconds": 2,
                }),
            );
            broker.broadcast_to_room_all(&p.room_id, &event);

            Frame::ok(
                req_id,
                serde_json::json!({
                    "room_id": p.room_id,
                    "candidates": candidates,
                    "message": "Election started. Agents have 2 seconds to decline.",
                }),
            )
        }
        Err(code) => Frame::error(
            req_id,
            ErrorPayload::new(code.clone(), format!("{:?}", code)),
        ),
    }
}

async fn handle_decline_election(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    vote_mgr: &Arc<VoteManager>,
) -> Frame {
    let p: DeclineElectionPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to decline election"),
        );
    }

    match vote_mgr.decline_election(&p.room_id, agent_id) {
        Ok(()) => Frame::ok(
            req_id,
            serde_json::json!({
                "room_id": p.room_id,
                "message": "Declined election",
            }),
        ),
        Err(code) => Frame::error(
            req_id,
            ErrorPayload::new(code.clone(), format!("{:?}", code)),
        ),
    }
}

async fn handle_decision(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    vote_mgr: &Arc<VoteManager>,
) -> Frame {
    let p: DecisionPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Must be the room leader
    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to issue decisions"),
        );
    }
    if !vote_mgr.is_leader(&p.room_id, agent_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(
                ErrorCode::NotLeader,
                "Only the room leader can issue decisions",
            ),
        );
    }

    if let Some(err) =
        reject_plaintext_in_encrypted_room(req_id, &p.content, &p.room_id, ephemeral_rooms, store)
    {
        return err;
    }

    let message_id = uuid::Uuid::new_v4().to_string();
    let is_ephemeral = ephemeral_rooms.contains_key(&p.room_id);

    // Persist as a special message if not ephemeral
    if !is_ephemeral {
        let decision_metadata = serde_json::json!({"type": "decision", "decided_by": agent_id});
        if let Err(e) = store.insert_message(
            &message_id,
            &p.room_id,
            agent_id,
            agent_name,
            &p.content,
            None,
            &decision_metadata,
        ) {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            );
        }
    }

    // Broadcast DecisionMade to room
    let event = Frame::event(
        FrameType::DecisionMade,
        serde_json::json!({
            "room_id": p.room_id,
            "message_id": message_id,
            "leader_id": agent_id,
            "leader_name": agent_name,
            "content": p.content,
            "metadata": p.metadata,
            "timestamp": chrono::Utc::now(),
        }),
    );
    broker.broadcast_to_room_all(&p.room_id, &event);

    Frame::ok(
        req_id,
        serde_json::json!({
            "message_id": message_id,
            "room_id": p.room_id,
        }),
    )
}

// --- Presence handlers ---

async fn handle_set_typing(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
) -> Frame {
    let p: SetTypingPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Not in this room"),
        );
    }

    // Update last_active
    if let Some(mut agent) = broker.agents.get_mut(agent_id) {
        agent.info.last_active = Some(chrono::Utc::now());
    }

    // Broadcast typing indicator to room (excluding sender)
    let event = Frame::event(
        FrameType::TypingIndicator,
        serde_json::json!({
            "room_id": p.room_id,
            "agent_id": agent_id,
            "agent_name": agent_name,
            "typing": p.typing,
        }),
    );
    broker.broadcast_to_room(&p.room_id, agent_id, &event);

    Frame::ok(req_id, serde_json::json!({"room_id": p.room_id}))
}

async fn handle_set_presence(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    agent_name: &str,
    broker: &Arc<Broker>,
) -> Frame {
    let p: SetPresencePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Validate status value
    match p.status.as_str() {
        "idle" | "waiting" | "working" | "thinking" => {}
        _ => {
            return Frame::error(
                req_id,
                ErrorPayload::new(
                    ErrorCode::InvalidPayload,
                    "status must be one of: idle, waiting, working, thinking",
                ),
            )
        }
    }

    // Update agent presence fields
    let rooms = {
        let mut agent = match broker.agents.get_mut(agent_id) {
            Some(a) => a,
            None => {
                return Frame::error(
                    req_id,
                    ErrorPayload::new(ErrorCode::NotRegistered, "Agent not found"),
                )
            }
        };
        agent.info.status = Some(p.status.clone());
        agent.info.status_detail = p.status_detail.clone();
        agent.info.progress = p.progress;
        agent.info.last_active = Some(chrono::Utc::now());
        agent.rooms.clone()
    };

    // Broadcast PresenceUpdate to all rooms the agent is in
    let event = Frame::event(
        FrameType::PresenceUpdate,
        serde_json::json!({
            "agent_id": agent_id,
            "agent_name": agent_name,
            "status": p.status,
            "status_detail": p.status_detail,
            "progress": p.progress,
        }),
    );
    for room_id in &rooms {
        broker.broadcast_to_room(room_id, agent_id, &event);
    }

    Frame::ok(req_id, serde_json::json!({"status": p.status}))
}

// --- Task handlers ---

async fn handle_assign_task(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    task_mgr: &Arc<TaskManager>,
) -> Frame {
    let p: AssignTaskPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &p.room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to assign tasks"),
        );
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let task = match task_mgr.create_task(
        task_id,
        p.room_id.clone(),
        p.title,
        p.description,
        p.assignee,
        agent_id.to_string(),
    ) {
        Ok(task) => task,
        Err(error) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, error.to_string()),
            )
        }
    };

    // Broadcast to room
    let event = Frame::event(
        FrameType::TaskAssigned,
        serde_json::to_value(&task).unwrap(),
    );
    broker.broadcast_to_room_all(&p.room_id, &event);

    Frame::ok(req_id, serde_json::to_value(&task).unwrap())
}

async fn handle_update_task(
    req_id: Option<&str>,
    payload: serde_json::Value,
    agent_id: &str,
    broker: &Arc<Broker>,
    task_mgr: &Arc<TaskManager>,
) -> Frame {
    let p: UpdateTaskPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    // Get the task to find its room
    let room_id = match task_mgr.get_task(&p.task_id) {
        Some(t) => t.room_id,
        None => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::TaskNotFound, "Task not found"),
            )
        }
    };

    if !broker.is_agent_in_room(agent_id, &room_id) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::NotInRoom, "Must be in room to update tasks"),
        );
    }

    match task_mgr.update_task(&p.task_id, p.status, p.assignee, p.note) {
        Ok(Some(task)) => {
            let event = Frame::event(FrameType::TaskUpdated, serde_json::to_value(&task).unwrap());
            broker.broadcast_to_room_all(&room_id, &event);
            Frame::ok(req_id, serde_json::to_value(&task).unwrap())
        }
        Ok(None) => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::TaskNotFound, "Task not found"),
        ),
        Err(error) => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InternalError, error.to_string()),
        ),
    }
}

async fn handle_list_tasks(
    req_id: Option<&str>,
    payload: serde_json::Value,
    task_mgr: &Arc<TaskManager>,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
) -> Frame {
    let p: ListTasksPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if let Err(error) = authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    let tasks = task_mgr.list_tasks(&p.room_id, p.status.as_deref());

    Frame {
        id: Some(uuid::Uuid::new_v4().to_string()),
        reply_to: req_id.map(String::from),
        frame_type: FrameType::TaskList,
        payload: serde_json::json!({
            "room_id": p.room_id,
            "tasks": tasks,
        }),
    }
}

// --- Webhook subscriptions ---

async fn handle_subscribe(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    ephemeral_rooms: &Arc<dashmap::DashMap<String, Room>>,
    agent_api_key: &str,
    no_auth: bool,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let p: SubscribePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };

    if let Err(error) = webhook_mgr.validate_url(&p.webhook_url).await {
        return Frame::error(req_id, ErrorPayload::new(ErrorCode::InvalidPayload, error));
    }
    if p.secret.is_empty() {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InvalidPayload, "secret is required"),
        );
    }

    if let Err(error) = authorize_room(
        req_id,
        &p.room_id,
        agent_api_key,
        no_auth,
        store,
        ephemeral_rooms,
    ) {
        return error;
    }

    // Resolve since_seq: explicit value, else current tip ("only new messages").
    let since_seq = match p.since_seq {
        Some(n) => n,
        None => store.room_tip(&p.room_id).unwrap_or(0),
    };

    let subscription_id = uuid::Uuid::new_v4().to_string();
    let owner_key = if no_auth { "no-auth" } else { agent_api_key };
    let sub = match store.create_subscription(
        &subscription_id,
        &p.room_id,
        owner_key,
        &p.webhook_url,
        &p.secret,
        &p.kinds,
        p.only_from.as_deref(),
        p.not_from.as_deref(),
        p.exclude_thinking,
        since_seq,
    ) {
        Ok(s) => s,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            )
        }
    };

    // Backfill: enqueue every chat message with seq > since_seq that matches.
    // Only walk the persisted room — ephemeral rooms don't have seek history
    // anyway, and a brand-new sub on an ephemeral room with since_seq=tip is a
    // no-op which is fine.
    let now = chrono::Utc::now();
    let mut backfill_cursor = since_seq;
    loop {
        let backlog =
            match store.get_history_filtered(&p.room_id, 1000, None, None, Some(backfill_cursor)) {
                Ok(backlog) => backlog,
                Err(_) => break,
            };
        if backlog.is_empty() {
            break;
        }
        for msg in &backlog {
            if !crate::webhooks::matches_filter(&sub, msg) {
                continue;
            }
            let delivery_id = uuid::Uuid::new_v4().to_string();
            let _ = store.enqueue_delivery(
                &delivery_id,
                &sub.subscription_id,
                msg.seq,
                &msg.message_id,
                now,
            );
        }
        backfill_cursor = backlog
            .last()
            .map(|message| message.seq)
            .unwrap_or(backfill_cursor);
        if backlog.len() < 1000 {
            break;
        }
    }
    webhook_mgr.wake();

    Frame::ok(req_id, serde_json::to_value(&sub).unwrap_or_default())
}

async fn handle_unsubscribe(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    agent_api_key: &str,
) -> Frame {
    let p: UnsubscribePayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };
    match store.delete_subscription(&p.subscription_id, agent_api_key) {
        Ok(true) => Frame::ok(req_id, serde_json::json!({"deleted": true})),
        Ok(false) => Frame::error(
            req_id,
            ErrorPayload::new(
                ErrorCode::AccessDenied,
                "Subscription not found or not yours",
            ),
        ),
        Err(e) => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
        ),
    }
}

async fn handle_list_subscriptions(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    agent_api_key: &str,
) -> Frame {
    let p: ListSubscriptionsPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };
    match store.list_subscriptions(agent_api_key, p.room_id.as_deref()) {
        Ok(subs) => Frame::ok(req_id, serde_json::json!({"subscriptions": subs})),
        Err(e) => Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
        ),
    }
}

async fn handle_enable_subscription(
    req_id: Option<&str>,
    payload: serde_json::Value,
    store: &Arc<Store>,
    agent_api_key: &str,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let p: EnableSubscriptionPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
            )
        }
    };
    let lookup = match store.get_subscription(&p.subscription_id) {
        Ok(Some(v)) => v,
        Ok(None) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::AccessDenied, "Subscription not found"),
            )
        }
        Err(e) => {
            return Frame::error(
                req_id,
                ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
            )
        }
    };
    let (sub, owner_key, _secret) = lookup;
    if owner_key != agent_api_key {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::AccessDenied, "Not your subscription"),
        );
    }
    if let Err(e) = store.set_subscription_status(&sub.subscription_id, "active", Some(0)) {
        return Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InternalError, e.to_string()),
        );
    }

    // Re-enqueue the backlog of messages past last_delivered_seq so the user
    // sees the events that piled up while the sub was failed.
    let now = chrono::Utc::now();
    if let Ok(backlog) =
        store.get_history_filtered(&sub.room_id, 1000, None, None, Some(sub.last_delivered_seq))
    {
        for msg in backlog {
            if !crate::webhooks::matches_filter(&sub, &msg) {
                continue;
            }
            let delivery_id = uuid::Uuid::new_v4().to_string();
            let _ = store.enqueue_delivery(
                &delivery_id,
                &sub.subscription_id,
                msg.seq,
                &msg.message_id,
                now,
            );
        }
    }
    webhook_mgr.wake();

    Frame::ok(
        req_id,
        serde_json::json!({"subscription_id": sub.subscription_id, "status": "active"}),
    )
}
