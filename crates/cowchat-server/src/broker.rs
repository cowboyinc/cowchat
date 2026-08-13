use cowchat_core::{ChatMessage, Frame, FrameType};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::connection::AgentConnection;

/// Result of a join: caller uses `new_holder` to decide whether to broadcast `turn_changed`.
#[derive(Debug)]
pub struct JoinOutcome {
    /// Set when the holder changed (i.e. room was empty and this agent became the holder).
    pub new_holder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRoomError {
    Destroyed,
}

const ROOM_TOMBSTONE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_ROOM_TOMBSTONES: usize = 4096;

struct RoomLifecycleState {
    tombstones: HashMap<String, Instant>,
}

impl RoomLifecycleState {
    fn new() -> Self {
        Self {
            tombstones: HashMap::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        self.tombstones
            .retain(|_, destroyed_at| now.duration_since(*destroyed_at) < ROOM_TOMBSTONE_TTL);
        while self.tombstones.len() > MAX_ROOM_TOMBSTONES {
            let Some(oldest) = self
                .tombstones
                .iter()
                .min_by_key(|(_, destroyed_at)| **destroyed_at)
                .map(|(room_id, _)| room_id.clone())
            else {
                break;
            };
            self.tombstones.remove(&oldest);
        }
    }

    fn tombstone(&mut self, room_id: &str) {
        let now = Instant::now();
        self.prune(now);
        self.tombstones.insert(room_id.to_string(), now);
        self.prune(now);
    }
}

/// Result of a leave: caller uses these to decide what to broadcast.
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
    /// Serializes room metadata mutations and their corresponding lifecycle
    /// events. Keeping this separate from `room_lifecycle` lets handlers
    /// publish without holding the admission/tombstone lock while still
    /// guaranteeing that create/rename/destroy events cannot overtake one
    /// another.
    room_mutation: Mutex<()>,
    /// Serializes same-identity registration/takeover with disconnect cleanup.
    /// This is deliberately separate from room lifecycle state so no agent-map
    /// shard is ever nested with room admission/destruction.
    agent_lifecycle: Mutex<()>,
    /// All connected agents: agent_id -> AgentConnection
    pub agents: Arc<DashMap<String, AgentConnection>>,
    /// Room membership in join order: room_id -> [agent_id, ...]
    pub room_members: Arc<DashMap<String, Vec<String>>>,
    /// Current turn-token holder per room: room_id -> agent_id.
    /// Absence = room is empty (no holder).
    turn_holders: DashMap<String, String>,
    /// UUIDs are never reused. A lifecycle tombstone closes the race where a
    /// join/reconnect observes a room immediately before its durable deletion
    /// and would otherwise recreate phantom in-memory membership afterwards.
    room_lifecycle: Mutex<RoomLifecycleState>,
}

impl Broker {
    pub fn new(
        agents: Arc<DashMap<String, AgentConnection>>,
        room_members: Arc<DashMap<String, Vec<String>>>,
    ) -> Self {
        Self {
            room_mutation: Mutex::new(()),
            agent_lifecycle: Mutex::new(()),
            agents,
            room_members,
            turn_holders: DashMap::new(),
            room_lifecycle: Mutex::new(RoomLifecycleState::new()),
        }
    }

    pub fn lock_agent_lifecycle(&self) -> std::sync::MutexGuard<'_, ()> {
        self.agent_lifecycle.lock().unwrap()
    }

    pub fn lock_room_mutation(&self) -> std::sync::MutexGuard<'_, ()> {
        self.room_mutation.lock().unwrap()
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
    pub fn join_room<F>(
        &self,
        agent_id: &str,
        room_id: &str,
        room_exists: F,
    ) -> Result<JoinOutcome, JoinRoomError>
    where
        F: FnOnce() -> bool,
    {
        // Existence validation and admission share the lifecycle lock with
        // room destruction. UUIDs are never reused, so a failed authoritative
        // lookup cannot later become the same room again.
        let mut lifecycle = self.room_lifecycle.lock().unwrap();
        lifecycle.prune(Instant::now());
        if lifecycle.tombstones.contains_key(room_id) || !room_exists() {
            return Err(JoinRoomError::Destroyed);
        }
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
        drop(lifecycle);
        Ok(JoinOutcome { new_holder })
    }

    /// Remove an agent from a room and update turn-token state. Leaving never
    /// destroys the room — rooms are removed only by explicit destroy.
    pub fn leave_room(&self, agent_id: &str, room_id: &str) -> LeaveOutcome {
        let mut lifecycle = self.room_lifecycle.lock().unwrap();
        lifecycle.prune(Instant::now());
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

        drop(lifecycle);
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

    /// Derive membership from its authoritative map. AgentConnection no longer
    /// carries a second, racy room cache.
    pub fn rooms_for_agent(&self, agent_id: &str) -> Vec<String> {
        self.room_members
            .iter()
            .filter(|entry| entry.value().iter().any(|member| member == agent_id))
            .map(|entry| entry.key().clone())
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

    /// Run authoritative deletion and remove every in-memory reference under
    /// the same lifecycle lock used by admission. The potentially blocking
    /// store callback runs before any broker map cleanup, but no agent-map lock
    /// is ever nested under this lifecycle lock.
    pub fn destroy_room_with<T, E, F>(
        &self,
        room_id: &str,
        delete_room: F,
    ) -> Result<(T, Vec<String>), E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let mut lifecycle = self.room_lifecycle.lock().unwrap();
        lifecycle.prune(Instant::now());
        let deleted = delete_room()?;
        lifecycle.tombstone(room_id);
        let members = self
            .room_members
            .remove(room_id)
            .map(|(_, members)| members)
            .unwrap_or_default();
        self.turn_holders.remove(room_id);
        drop(lifecycle);
        Ok((deleted, members))
    }

    /// Tombstone broker-only state when the authoritative metadata was already
    /// removed by an atomic lifecycle operation.
    pub fn destroy_room(&self, room_id: &str) -> Vec<String> {
        self.destroy_room_with(room_id, || Ok::<(), std::convert::Infallible>(()))
            .expect("infallible room cleanup")
            .1
    }

    pub fn is_room_destroyed(&self, room_id: &str) -> bool {
        let mut lifecycle = self.room_lifecycle.lock().unwrap();
        lifecycle.prune(Instant::now());
        lifecycle.tombstones.contains_key(room_id)
    }

    #[cfg(test)]
    fn tombstone_count(&self) -> usize {
        let mut lifecycle = self.room_lifecycle.lock().unwrap();
        lifecycle.prune(Instant::now());
        lifecycle.tombstones.len()
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

    #[test]
    fn destruction_tombstone_wins_against_concurrent_joins() {
        let broker = Arc::new(Broker::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        ));
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let mut joins = Vec::new();
        for index in 0..16 {
            let broker = broker.clone();
            let barrier = barrier.clone();
            joins.push(std::thread::spawn(move || {
                barrier.wait();
                let _ = broker.join_room(&format!("agent-{index}"), "doomed", || true);
            }));
        }

        barrier.wait();
        broker.destroy_room("doomed");
        for join in joins {
            join.join().unwrap();
        }

        assert!(broker.is_room_destroyed("doomed"));
        assert!(broker.get_room_members("doomed").is_empty());
        assert_eq!(
            broker.join_room("late", "doomed", || true).unwrap_err(),
            JoinRoomError::Destroyed
        );
    }

    #[test]
    fn persistent_destroy_commit_serializes_a_waiting_join() {
        let broker = Arc::new(Broker::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        ));
        broker.join_room("member", "persistent", || true).unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (commit_tx, commit_rx) = std::sync::mpsc::channel();
        let destroy_broker = broker.clone();
        let destroy = std::thread::spawn(move || {
            destroy_broker.destroy_room_with("persistent", || {
                entered_tx.send(()).unwrap();
                commit_rx.recv().unwrap();
                Ok::<_, ()>(())
            })
        });
        entered_rx.recv().unwrap();
        let join_broker = broker.clone();
        let join = std::thread::spawn(move || join_broker.join_room("late", "persistent", || true));
        commit_tx.send(()).unwrap();

        destroy.join().unwrap().unwrap();
        assert_eq!(join.join().unwrap().unwrap_err(), JoinRoomError::Destroyed);
        assert!(broker.get_room_members("persistent").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn destruction_does_not_hold_lifecycle_lock_while_waiting_on_agents() {
        let agents = Arc::new(DashMap::new());
        let broker = Arc::new(Broker::new(agents.clone(), Arc::new(DashMap::new())));
        let (sender, _receiver) = mpsc::channel(1);
        agents.insert(
            "same".into(),
            AgentConnection::new(
                AgentInfo {
                    agent_id: "same".into(),
                    name: "same".into(),
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
                Arc::new(Notify::new()),
                "key".into(),
            ),
        );
        let _agent_guard = agents.get_mut("same").unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let destroy_broker = broker.clone();
        std::thread::spawn(move || {
            destroy_broker.destroy_room("doomed");
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("destroy must not wait on an agent shard while holding lifecycle state");
        assert!(broker.is_room_destroyed("doomed"));
    }

    #[test]
    fn lifecycle_tombstones_are_bounded_under_churn() {
        let broker = Broker::new(Arc::new(DashMap::new()), Arc::new(DashMap::new()));
        for index in 0..(MAX_ROOM_TOMBSTONES * 2) {
            broker.destroy_room(&format!("destroyed-{index}"));
        }
        assert!(broker.tombstone_count() <= MAX_ROOM_TOMBSTONES);
        assert_eq!(
            broker
                .join_room("late", "destroyed-0", || false)
                .unwrap_err(),
            JoinRoomError::Destroyed,
            "authoritative absence still rejects admission after cache eviction"
        );
    }
}
