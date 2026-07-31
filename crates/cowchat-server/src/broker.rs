use cowchat_core::{ChatMessage, Frame, FrameType};
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::connection::AgentConnection;

/// Result of a join: caller uses `new_holder` to decide whether to broadcast `turn_changed`.
pub struct JoinOutcome {
    /// Set when the holder changed (i.e. room was empty and this agent became the holder).
    pub new_holder: Option<String>,
}

/// Result of a leave: caller uses these to decide whether to clean up and what to broadcast.
pub struct LeaveOutcome {
    pub now_empty: bool,
    /// Set when this leave caused the holder to change. None means holder is unchanged
    /// OR the room is now empty (check `now_empty` to disambiguate).
    pub new_holder: Option<String>,
    pub holder_changed: bool,
}

/// Routes messages to room members and handles @mentions.
///
/// Membership is stored as a `Vec<String>` keyed by room_id so that join order is preserved
/// — this is the basis for the round-robin turn token. The token holder for each room is
/// tracked in `turn_holders`; absence means the room is currently empty.
pub struct Broker {
    /// All connected agents: agent_id -> AgentConnection
    pub agents: Arc<DashMap<String, AgentConnection>>,
    /// Room membership in join order: room_id -> [agent_id, ...]
    pub room_members: Arc<DashMap<String, Vec<String>>>,
    /// Current turn-token holder per room: room_id -> agent_id.
    /// Absence = room is empty (no holder).
    turn_holders: DashMap<String, String>,
    /// Per-room monotonic seq counter for ephemeral rooms (permanent rooms use SQLite).
    ephemeral_seqs: DashMap<String, AtomicI64>,
}

impl Broker {
    pub fn new(
        agents: Arc<DashMap<String, AgentConnection>>,
        room_members: Arc<DashMap<String, Vec<String>>>,
    ) -> Self {
        Self {
            agents,
            room_members,
            turn_holders: DashMap::new(),
            ephemeral_seqs: DashMap::new(),
        }
    }

    /// Allocate the next seq for an ephemeral room. Starts at 1.
    pub fn next_ephemeral_seq(&self, room_id: &str) -> i64 {
        let entry = self
            .ephemeral_seqs
            .entry(room_id.to_string())
            .or_insert_with(|| AtomicI64::new(0));
        entry.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Read the current seq for an ephemeral room without advancing it.
    pub fn ephemeral_room_tip(&self, room_id: &str) -> i64 {
        self.ephemeral_seqs
            .get(room_id)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Drop the seq counter for a room (e.g. when an ephemeral room is destroyed).
    pub fn forget_ephemeral_seq(&self, room_id: &str) {
        self.ephemeral_seqs.remove(room_id);
    }

    /// Broadcast a message to all members of a room, except the sender.
    pub fn broadcast_to_room(&self, room_id: &str, sender_id: &str, frame: &Frame) {
        if let Some(members) = self.room_members.get(room_id) {
            for member_id in members.iter() {
                if member_id == sender_id {
                    continue;
                }
                self.send_to_agent(member_id, frame.clone());
            }
        }
    }

    /// Broadcast a frame to ALL members of a room (including sender).
    pub fn broadcast_to_room_all(&self, room_id: &str, frame: &Frame) {
        if let Some(members) = self.room_members.get(room_id) {
            for member_id in members.iter() {
                self.send_to_agent(member_id, frame.clone());
            }
        }
    }

    /// Send a mention notification only to current room members. A client may
    /// submit arbitrary public agent ids, but must not exfiltrate private room
    /// content by mentioning an outsider.
    pub fn send_mentions(&self, mentions: &[String], message: &ChatMessage, room_id: &str) {
        let frame = Frame::event(
            FrameType::Mention,
            serde_json::json!({
                "room_id": room_id,
                "message": message,
            }),
        );

        for agent_id in mentions {
            if self.is_agent_in_room(agent_id, room_id) {
                self.send_to_agent(agent_id, frame.clone());
            }
        }
    }

    /// Send a frame to a specific agent by ID.
    pub fn send_to_agent(&self, agent_id: &str, frame: Frame) {
        if let Some(agent) = self.agents.get(agent_id) {
            if let Err(error) = agent.sender.try_send(frame) {
                log::warn!(
                    "Disconnecting lagging agent {}: bounded queue unavailable: {}",
                    agent_id,
                    error
                );
                agent.disconnect.notify_one();
            }
        }
    }

    /// Check if an agent is in a specific room.
    pub fn is_agent_in_room(&self, agent_id: &str, room_id: &str) -> bool {
        self.room_members
            .get(room_id)
            .map(|members| members.iter().any(|m| m == agent_id))
            .unwrap_or(false)
    }

    /// Add an agent to a room and update turn-token state.
    ///
    /// If the room was empty (no holder), the joining agent becomes the holder; the caller
    /// should broadcast `turn_changed`. Otherwise the holder is unchanged.
    pub fn join_room(&self, agent_id: &str, room_id: &str) -> JoinOutcome {
        let mut entry = self.room_members.entry(room_id.to_string()).or_default();
        if !entry.iter().any(|m| m == agent_id) {
            entry.push(agent_id.to_string());
        }
        drop(entry);

        let new_holder = if !self.turn_holders.contains_key(room_id) {
            self.turn_holders
                .insert(room_id.to_string(), agent_id.to_string());
            Some(agent_id.to_string())
        } else {
            None
        };
        JoinOutcome { new_holder }
    }

    /// Remove an agent from a room and update turn-token state.
    pub fn leave_room(&self, agent_id: &str, room_id: &str) -> LeaveOutcome {
        let (now_empty, was_holder) = {
            let was_holder = self
                .turn_holders
                .get(room_id)
                .map(|h| *h == *agent_id)
                .unwrap_or(false);

            let mut now_empty = true;
            if let Some(mut members) = self.room_members.get_mut(room_id) {
                members.retain(|m| m != agent_id);
                now_empty = members.is_empty();
            }
            (now_empty, was_holder)
        };

        let (new_holder, holder_changed) = if was_holder {
            if now_empty {
                self.turn_holders.remove(room_id);
                (None, true)
            } else {
                let next = self
                    .room_members
                    .get(room_id)
                    .and_then(|m| m.first().cloned());
                if let Some(next_id) = next.clone() {
                    self.turn_holders.insert(room_id.to_string(), next_id);
                } else {
                    self.turn_holders.remove(room_id);
                }
                (next, true)
            }
        } else {
            (None, false)
        };

        LeaveOutcome {
            now_empty,
            new_holder,
            holder_changed,
        }
    }

    /// Remove an agent from all rooms. Returns one outcome per affected room.
    pub fn leave_all_rooms(&self, agent_id: &str) -> Vec<(String, LeaveOutcome)> {
        let room_ids: Vec<String> = self
            .room_members
            .iter()
            .filter(|entry| entry.value().iter().any(|m| m == agent_id))
            .map(|entry| entry.key().clone())
            .collect();

        room_ids
            .into_iter()
            .map(|room_id| {
                let outcome = self.leave_room(agent_id, &room_id);
                (room_id, outcome)
            })
            .collect()
    }

    /// Get all agent IDs in a room, in join order.
    pub fn get_room_members(&self, room_id: &str) -> Vec<String> {
        self.room_members
            .get(room_id)
            .map(|members| members.clone())
            .unwrap_or_default()
    }

    /// Current turn-token holder for a room, if any.
    pub fn turn_holder(&self, room_id: &str) -> Option<String> {
        self.turn_holders.get(room_id).map(|h| h.clone())
    }

    /// Advance the turn token to the next member after `sender_id` in join order.
    ///
    /// Called after a successful `send_message` to publish "whoever spoke last passes
    /// to the next." Works whether or not the sender was the previous holder — under
    /// advisory semantics anyone in the room can send. With a single member, the holder
    /// is unchanged (they keep the token). Returns the new holder, or None if the room
    /// is empty.
    pub fn advance_turn_from(&self, room_id: &str, sender_id: &str) -> Option<String> {
        let members = self.get_room_members(room_id);
        if members.is_empty() {
            self.turn_holders.remove(room_id);
            return None;
        }

        let next = match members.iter().position(|m| *m == sender_id) {
            Some(i) => members[(i + 1) % members.len()].clone(),
            // Sender is in the room (we checked earlier) but somehow not in the order
            // — fall back to the first member.
            None => members[0].clone(),
        };
        self.turn_holders.insert(room_id.to_string(), next.clone());
        Some(next)
    }

    /// Remove the membership entry for a room entirely.
    pub fn remove_room(&self, room_id: &str) {
        self.room_members.remove(room_id);
        self.turn_holders.remove(room_id);
    }

    /// Get sender channel for a new agent connection.
    pub fn create_agent_channel(&self) -> (mpsc::Sender<Frame>, mpsc::Receiver<Frame>) {
        mpsc::channel(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowchat_core::AgentInfo;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn saturated_outbound_queue_disconnects_lagging_agent() {
        let agents = Arc::new(DashMap::new());
        let broker = Broker::new(agents.clone(), Arc::new(DashMap::new()));
        let (sender, _receiver) = mpsc::channel(1);
        let disconnect = Arc::new(Notify::new());
        agents.insert(
            "slow".into(),
            AgentConnection::new(
                AgentInfo {
                    agent_id: "slow".into(),
                    name: "slow".into(),
                    capabilities: vec![],
                    connected_at: None,
                    last_active: None,
                    status: None,
                    status_detail: None,
                    progress: None,
                },
                "session".into(),
                sender,
                tokio::spawn(async {}),
                tokio::spawn(async {}),
                disconnect.clone(),
                "key".into(),
            ),
        );
        let event = Frame::event(FrameType::Ping, serde_json::json!({}));
        broker.send_to_agent("slow", event.clone());
        broker.send_to_agent("slow", event);
        tokio::time::timeout(std::time::Duration::from_millis(100), disconnect.notified())
            .await
            .expect("queue saturation must signal disconnect");
    }
}
