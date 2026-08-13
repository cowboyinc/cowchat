use cowchat_core::Frame;
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::connection::can_access_room_parts;

/// How long we keep a disconnected agent's state before discarding it.
const RECONNECT_WINDOW_SECS: u64 = 120;

/// Maximum messages buffered per disconnected agent.
const MAX_BUFFERED_MESSAGES: usize = 200;
const DESTROYED_ROOM_TTL: Duration = Duration::from_secs(RECONNECT_WINDOW_SECS + 60);
const MAX_DESTROYED_ROOMS: usize = 4096;

struct DestroyedRoomCache {
    rooms: HashMap<String, Instant>,
}

impl DestroyedRoomCache {
    fn new() -> Self {
        Self {
            rooms: HashMap::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        self.rooms
            .retain(|_, destroyed_at| now.duration_since(*destroyed_at) < DESTROYED_ROOM_TTL);
        while self.rooms.len() > MAX_DESTROYED_ROOMS {
            let Some(oldest) = self
                .rooms
                .iter()
                .min_by_key(|(_, destroyed_at)| **destroyed_at)
                .map(|(room_id, _)| room_id.clone())
            else {
                break;
            };
            self.rooms.remove(&oldest);
        }
    }

    fn insert(&mut self, room_id: &str) {
        let now = Instant::now();
        self.prune(now);
        self.rooms.insert(room_id.to_string(), now);
        self.prune(now);
    }

    fn contains(&mut self, room_id: &str) -> bool {
        self.prune(Instant::now());
        self.rooms.contains_key(room_id)
    }
}

/// State stashed when an agent disconnects, allowing seamless reconnect.
#[derive(Clone)]
pub struct StashedAgent {
    pub agent_id: String,
    pub name: String,
    pub api_key: String,
    pub rooms: HashSet<String>,
    pub missed_messages: Vec<Frame>,
    pub disconnected_at: Instant,
    reclaim_token: Option<String>,
}

/// Manages reconnect state for recently disconnected agents.
#[derive(Clone)]
pub struct ReconnectManager {
    stashed: Arc<DashMap<String, StashedAgent>>,
    destroyed_rooms: Arc<Mutex<DestroyedRoomCache>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimError {
    CredentialMismatch,
    AlreadyClaimed,
}

/// Exclusive, abort-safe ownership of a reconnect stash. The entry remains in
/// the manager while registration is pending, so room metadata events can
/// still be buffered. Dropping an uncommitted lease releases it for retry.
pub struct ReclaimLease {
    manager: ReconnectManager,
    agent_id: String,
    token: String,
    active: bool,
}

impl ReclaimLease {
    pub fn snapshot(&self) -> Option<StashedAgent> {
        self.manager
            .stashed
            .get(&self.agent_id)
            .filter(|entry| entry.reclaim_token.as_deref() == Some(self.token.as_str()))
            .map(|entry| entry.clone())
    }

    pub fn commit(mut self) -> Option<StashedAgent> {
        let removed = self
            .manager
            .stashed
            .remove_if(&self.agent_id, |_, entry| {
                entry.reclaim_token.as_deref() == Some(self.token.as_str())
            })
            .map(|(_, mut entry)| {
                entry.reclaim_token = None;
                entry
            });
        self.active = false;
        removed
    }
}

impl Drop for ReclaimLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(mut entry) = self.manager.stashed.get_mut(&self.agent_id) {
            if entry.reclaim_token.as_deref() == Some(self.token.as_str()) {
                entry.reclaim_token = None;
            }
        }
    }
}

impl ReconnectManager {
    pub fn new() -> Self {
        let mgr = Self {
            stashed: Arc::new(DashMap::new()),
            destroyed_rooms: Arc::new(Mutex::new(DestroyedRoomCache::new())),
        };

        // Spawn background cleanup task
        let stashed = mgr.stashed.clone();
        let destroyed_rooms = mgr.destroyed_rooms.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let cutoff = Duration::from_secs(RECONNECT_WINDOW_SECS);
                stashed.retain(|_, v| v.disconnected_at.elapsed() < cutoff);
                destroyed_rooms.lock().unwrap().prune(Instant::now());
            }
        });

        mgr
    }

    /// Stash an agent's state on disconnect. Returns immediately.
    pub fn stash(
        &self,
        agent_id: String,
        name: String,
        api_key: String,
        mut rooms: HashSet<String>,
    ) {
        rooms.retain(|room_id| !self.room_is_destroyed(room_id));
        log::info!(
            "Stashing reconnect state for {} ({}) — {} rooms",
            name,
            agent_id,
            rooms.len()
        );
        let stash_key = agent_id.clone();
        self.stashed.insert(
            stash_key.clone(),
            StashedAgent {
                agent_id,
                name,
                api_key,
                rooms,
                missed_messages: Vec::new(),
                disconnected_at: Instant::now(),
                reclaim_token: None,
            },
        );
        // Double-check after insertion. If destruction raced between the first
        // filter and the map insert, its tombstone is now visible; if it starts
        // after this pass, `forget_room` will see and clean this stash.
        if let Some(mut stashed) = self.stashed.get_mut(&stash_key) {
            stashed
                .rooms
                .retain(|room_id| !self.room_is_destroyed(room_id));
        }
    }

    fn room_is_destroyed(&self, room_id: &str) -> bool {
        self.destroyed_rooms.lock().unwrap().contains(room_id)
    }

    /// Begin an exclusive reconnect claim without removing the stash. This
    /// closes the interval where the agent was neither live nor bufferable.
    pub fn begin_reclaim(
        &self,
        agent_id: &str,
        api_key: &str,
        no_auth: bool,
    ) -> Result<Option<ReclaimLease>, ReclaimError> {
        let Some(mut stashed) = self.stashed.get_mut(agent_id) else {
            return Ok(None);
        };
        if stashed.disconnected_at.elapsed() >= Duration::from_secs(RECONNECT_WINDOW_SECS) {
            drop(stashed);
            self.stashed.remove(agent_id);
            return Ok(None);
        }
        if !no_auth && stashed.api_key != api_key {
            return Err(ReclaimError::CredentialMismatch);
        }
        if stashed.reclaim_token.is_some() {
            return Err(ReclaimError::AlreadyClaimed);
        }
        let token = uuid::Uuid::new_v4().to_string();
        stashed.reclaim_token = Some(token.clone());
        drop(stashed);
        Ok(Some(ReclaimLease {
            manager: self.clone(),
            agent_id: agent_id.to_string(),
            token,
            active: true,
        }))
    }

    /// Try to reclaim a stashed agent. A stable identity is owned by the API
    /// key that created it; another credential must not be able to reclaim it.
    pub fn reclaim(
        &self,
        agent_id: &str,
        api_key: &str,
        no_auth: bool,
    ) -> Result<Option<StashedAgent>, ReclaimError> {
        let Some(lease) = self.begin_reclaim(agent_id, api_key, no_auth)? else {
            return Ok(None);
        };
        let stashed = lease.commit();
        if let Some(stashed) = &stashed {
            log::info!(
                "Agent {} ({}) reclaimed after {:.1}s",
                stashed.name,
                agent_id,
                stashed.disconnected_at.elapsed().as_secs_f64()
            );
        }
        Ok(stashed)
    }

    /// Buffer a message for a disconnected agent (e.g. room messages they're missing).
    pub fn buffer_message(&self, agent_id: &str, frame: Frame) {
        if frame_room_id(&frame).is_some_and(|room_id| self.room_is_destroyed(room_id)) {
            return;
        }
        if let Some(mut stashed) = self.stashed.get_mut(agent_id) {
            if stashed.missed_messages.len() < MAX_BUFFERED_MESSAGES {
                stashed.missed_messages.push(frame);
            }
        }
    }

    /// Buffer room metadata changes for disconnected agents that are allowed
    /// to discover the room. Public room updates go to every stash; private
    /// updates remain confined to the owning API key plus any keys in
    /// `granted_keys` (invite-issued room grants).
    pub fn buffer_visible_room_event(
        &self,
        visibility: &str,
        owner_key: Option<&str>,
        granted_keys: &HashSet<String>,
        no_auth: bool,
        frame: &Frame,
    ) -> HashSet<String> {
        let mut buffered = HashSet::new();
        for mut entry in self.stashed.iter_mut() {
            if (can_access_room_parts(visibility, owner_key, &entry.api_key, no_auth)
                || (!entry.api_key.is_empty() && granted_keys.contains(&entry.api_key)))
                && frame_room_id(frame).is_none_or(|room_id| !self.room_is_destroyed(room_id))
                && entry.missed_messages.len() < MAX_BUFFERED_MESSAGES
            {
                entry.missed_messages.push(frame.clone());
                buffered.insert(entry.key().clone());
            }
        }
        buffered
    }

    /// Check if an agent_id is stashed (recently disconnected, awaiting reconnect).
    pub fn is_stashed(&self, agent_id: &str) -> bool {
        self.stashed.contains_key(agent_id)
    }

    /// Get the set of stashed agent IDs that are members of a given room.
    pub fn stashed_members_of_room(&self, room_id: &str) -> Vec<String> {
        self.stashed
            .iter()
            .filter(|entry| entry.value().rooms.contains(room_id))
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Remove a destroyed room from every reconnect stash and discard buffered
    /// events for it. A reconnect must never resurrect membership or replay
    /// room data after the room lifecycle has ended.
    pub fn forget_room(&self, room_id: &str) {
        self.destroyed_rooms.lock().unwrap().insert(room_id);
        for mut entry in self.stashed.iter_mut() {
            entry.rooms.remove(room_id);
            entry
                .missed_messages
                .retain(|frame| frame_room_id(frame) != Some(room_id));
        }
    }

    #[cfg(test)]
    fn destroyed_room_count(&self) -> usize {
        let mut cache = self.destroyed_rooms.lock().unwrap();
        cache.prune(Instant::now());
        cache.rooms.len()
    }
}

fn frame_room_id(frame: &Frame) -> Option<&str> {
    frame
        .payload
        .get("room_id")
        .and_then(|value| value.as_str())
        .or_else(|| {
            frame
                .payload
                .get("message")
                .and_then(|value| value.get("room_id"))
                .and_then(|value| value.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn expired_stash_does_not_define_identity_ownership() {
        let manager = ReconnectManager::new();
        manager.stash(
            "stable".into(),
            "Stable".into(),
            "owner-key".into(),
            HashSet::new(),
        );
        manager.stashed.get_mut("stable").unwrap().disconnected_at =
            Instant::now() - Duration::from_secs(RECONNECT_WINDOW_SECS + 1);
        assert!(manager
            .reclaim("stable", "owner-key", false)
            .unwrap()
            .is_none());
        // Permanent ownership is intentionally tested in Store/server restart
        // tests; this manager only owns the short-lived room/message stash.
    }

    #[tokio::test]
    async fn destroyed_room_is_removed_from_reconnect_state() {
        let manager = ReconnectManager::new();
        manager.stash(
            "stable".into(),
            "Stable".into(),
            "owner-key".into(),
            HashSet::from(["keep".into(), "destroyed".into()]),
        );
        manager.buffer_message(
            "stable",
            Frame::event(
                cowchat_core::FrameType::MessageReceived,
                serde_json::json!({"room_id": "destroyed", "content": "secret"}),
            ),
        );
        manager.buffer_message(
            "stable",
            Frame::event(
                cowchat_core::FrameType::MessageReceived,
                serde_json::json!({"room_id": "keep", "content": "keep"}),
            ),
        );

        manager.forget_room("destroyed");
        let stashed = manager
            .reclaim("stable", "owner-key", false)
            .unwrap()
            .unwrap();
        assert_eq!(stashed.rooms, HashSet::from(["keep".into()]));
        assert_eq!(stashed.missed_messages.len(), 1);
        assert_eq!(stashed.missed_messages[0].payload["room_id"], "keep");
    }

    #[tokio::test]
    async fn stash_created_after_destruction_cannot_restore_room() {
        let manager = ReconnectManager::new();
        manager.forget_room("destroyed");
        manager.stash(
            "late".into(),
            "Late".into(),
            "owner-key".into(),
            HashSet::from(["keep".into(), "destroyed".into()]),
        );
        manager.buffer_message(
            "late",
            Frame::event(
                cowchat_core::FrameType::MessageReceived,
                serde_json::json!({"room_id": "destroyed", "content": "secret"}),
            ),
        );
        let stashed = manager
            .reclaim("late", "owner-key", false)
            .unwrap()
            .unwrap();
        assert_eq!(stashed.rooms, HashSet::from(["keep".into()]));
        assert!(stashed.missed_messages.is_empty());
    }

    #[tokio::test]
    async fn room_metadata_events_are_buffered_by_visibility() {
        let manager = ReconnectManager::new();
        for (agent_id, api_key) in [("owner", "owner-key"), ("outsider", "other-key")] {
            manager.stash(
                agent_id.into(),
                agent_id.into(),
                api_key.into(),
                HashSet::from(["persistent-room".into()]),
            );
        }
        let private_event = Frame::event(
            cowchat_core::FrameType::RoomUpdated,
            serde_json::json!({"room_id": "private-room", "name": "renamed-private"}),
        );
        manager.buffer_visible_room_event(
            "private",
            Some("owner-key"),
            &HashSet::new(),
            false,
            &private_event,
        );

        let owner = manager
            .reclaim("owner", "owner-key", false)
            .unwrap()
            .unwrap();
        assert_eq!(owner.missed_messages.len(), 1);
        let outsider = manager
            .reclaim("outsider", "other-key", false)
            .unwrap()
            .unwrap();
        assert!(outsider.missed_messages.is_empty());

        for (agent_id, api_key) in [("owner", "owner-key"), ("outsider", "other-key")] {
            manager.stash(
                agent_id.into(),
                agent_id.into(),
                api_key.into(),
                HashSet::from(["persistent-room".into()]),
            );
        }
        let public_event = Frame::event(
            cowchat_core::FrameType::RoomUpdated,
            serde_json::json!({"room_id": "public-room", "name": "renamed-public"}),
        );
        manager.buffer_visible_room_event(
            "public",
            Some("owner-key"),
            &HashSet::new(),
            false,
            &public_event,
        );
        for (agent_id, api_key) in [("owner", "owner-key"), ("outsider", "other-key")] {
            let stashed = manager.reclaim(agent_id, api_key, false).unwrap().unwrap();
            assert_eq!(stashed.missed_messages.len(), 1);
        }
    }

    #[tokio::test]
    async fn reconnect_lease_is_exclusive_bufferable_and_abort_safe() {
        let manager = ReconnectManager::new();
        manager.stash(
            "stable".into(),
            "Stable".into(),
            "owner-key".into(),
            HashSet::from(["room".into()]),
        );

        let first = manager
            .begin_reclaim("stable", "owner-key", false)
            .unwrap()
            .unwrap();
        assert!(matches!(
            manager.begin_reclaim("stable", "owner-key", false),
            Err(ReclaimError::AlreadyClaimed)
        ));
        let event = Frame::event(
            cowchat_core::FrameType::RoomUpdated,
            serde_json::json!({"room_id": "room", "name": "during-handshake"}),
        );
        let buffered = manager.buffer_visible_room_event(
            "private",
            Some("owner-key"),
            &HashSet::new(),
            false,
            &event,
        );
        assert_eq!(buffered, HashSet::from(["stable".to_string()]));

        // A failed handshake drops the lease without consuming state.
        drop(first);
        let retry = manager
            .begin_reclaim("stable", "owner-key", false)
            .unwrap()
            .unwrap();
        let reclaimed = retry.commit().unwrap();
        assert_eq!(reclaimed.rooms, HashSet::from(["room".into()]));
        assert_eq!(reclaimed.missed_messages.len(), 1);
        assert_eq!(
            reclaimed.missed_messages[0].payload["name"],
            event.payload["name"]
        );
    }

    #[tokio::test]
    async fn destruction_during_reconnect_lease_cannot_be_reported_as_restored() {
        let manager = ReconnectManager::new();
        manager.stash(
            "stable".into(),
            "Stable".into(),
            "owner-key".into(),
            HashSet::from(["destroyed".into()]),
        );
        let lease = manager
            .begin_reclaim("stable", "owner-key", false)
            .unwrap()
            .unwrap();
        assert!(lease.snapshot().unwrap().rooms.contains("destroyed"));

        manager.forget_room("destroyed");
        let pending = lease.snapshot().unwrap();
        assert!(!pending.rooms.contains("destroyed"));
        assert!(pending.missed_messages.is_empty());
        let committed = lease.commit().unwrap();
        assert!(!committed.rooms.contains("destroyed"));
    }

    #[tokio::test]
    async fn ownerless_private_events_are_buffered_only_for_keyless_stashes() {
        let manager = ReconnectManager::new();
        manager.stash(
            "keyless".into(),
            "Keyless".into(),
            String::new(),
            HashSet::new(),
        );
        manager.stash(
            "authenticated".into(),
            "Authenticated".into(),
            "owner-key".into(),
            HashSet::new(),
        );
        let event = Frame::event(
            cowchat_core::FrameType::RoomUpdated,
            serde_json::json!({"room_id": "ownerless"}),
        );
        let buffered =
            manager.buffer_visible_room_event("private", None, &HashSet::new(), false, &event);
        assert_eq!(buffered, HashSet::from(["keyless".to_string()]));
    }

    #[tokio::test]
    async fn granted_keys_receive_private_room_events_keyless_stashes_do_not() {
        let manager = ReconnectManager::new();
        for (agent_id, api_key) in [
            ("granted", "granted-key"),
            ("outsider", "other-key"),
            ("keyless", ""),
        ] {
            manager.stash(
                agent_id.into(),
                agent_id.into(),
                api_key.into(),
                HashSet::new(),
            );
        }
        let event = Frame::event(
            cowchat_core::FrameType::RoomUpdated,
            serde_json::json!({"room_id": "invited-room"}),
        );
        let granted = HashSet::from(["granted-key".to_string(), String::new()]);
        let buffered = manager.buffer_visible_room_event(
            "private",
            Some("owner-key"),
            &granted,
            false,
            &event,
        );
        // An empty entry in the granted set must never widen keyless access.
        assert_eq!(buffered, HashSet::from(["granted".to_string()]));
    }

    #[tokio::test]
    async fn destroyed_room_cache_is_bounded_and_prunable() {
        let manager = ReconnectManager::new();
        for index in 0..(MAX_DESTROYED_ROOMS * 2) {
            manager.forget_room(&format!("destroyed-{index}"));
        }
        assert!(manager.destroyed_room_count() <= MAX_DESTROYED_ROOMS);

        {
            let mut cache = manager.destroyed_rooms.lock().unwrap();
            for destroyed_at in cache.rooms.values_mut() {
                *destroyed_at = Instant::now() - DESTROYED_ROOM_TTL - Duration::from_secs(1);
            }
        }
        assert_eq!(manager.destroyed_room_count(), 0);
    }
}
