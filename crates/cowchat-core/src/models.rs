use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<DateTime<Utc>>,
    /// Last time this agent sent a message or typed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active: Option<DateTime<Utc>>,
    /// Presence status: "idle", "waiting", "working", "thinking"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Human-readable detail, e.g. "reviewing section 3"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    /// Progress percentage 0-100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub room_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub ephemeral: bool,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Room visibility: "public" or "private". Private rooms are only visible to the owning key.
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// API key that owns this room (None for system rooms like lobby).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<String>,
    /// Most recent message timestamp in this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<DateTime<Utc>>,
    /// Number of agents currently in this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<usize>,
    /// End-to-end encrypted room. Message `content` is ciphertext the server
    /// cannot read; clients must hold the pre-shared room key. The server
    /// rejects plaintext sends to encrypted rooms (see crypto module).
    #[serde(default)]
    pub encrypted: bool,
}

fn default_visibility() -> String {
    "private".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub message_id: String,
    pub room_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub timestamp: DateTime<Utc>,
    /// Per-room monotonic sequence number assigned by the server.
    /// Permanent rooms persist this to SQLite; ephemeral rooms maintain it in-memory.
    /// Use this with `room_tip` and `--since-seq` to detect "have I seen the latest?".
    #[serde(default)]
    pub seq: i64,
}

// --- Command payloads (client -> server) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterPayload {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// If true and agent_id matches a recently disconnected agent, restore room
    /// memberships and replay missed messages (IRC bouncer behavior).
    #[serde(default)]
    pub reconnect: bool,
    /// Wire protocol version the client speaks (`PROTOCOL_VERSION`). Absent on
    /// pre-versioning clients, which the server treats as version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomPayload {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub ephemeral: bool,
    /// If true, room is public (any API key can join). Default: private.
    #[serde(default)]
    pub public: bool,
    /// If true, the room is end-to-end encrypted: the server rejects plaintext
    /// `content` and only relays `clw1:` ciphertext blobs. Agents must share a
    /// pre-shared room key to read messages.
    #[serde(default)]
    pub encrypted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRoomPayload {
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveRoomPayload {
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessagePayload {
    pub room_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetHistoryPayload {
    pub room_id: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    /// Return only messages after this message_id (exclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Return only messages with seq strictly greater than this value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<i64>,
}

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRoomsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAgentsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfoPayload {
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomTipPayload {
    pub room_id: String,
}

/// Latest seq for a room. `seq` is 0 if the room has no messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomTipResultPayload {
    pub room_id: String,
    pub seq: i64,
}

// --- Voting payloads ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVotePayload {
    pub room_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub options: Vec<String>,
    /// Deadline in seconds from now. If None, vote stays open until all members vote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastVotePayload {
    pub vote_id: String,
    /// Index into the options list (0-based).
    pub option_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetVoteStatusPayload {
    pub vote_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListVotesPayload {
    pub room_id: String,
    #[serde(default = "default_vote_limit")]
    pub limit: u32,
}

fn default_vote_limit() -> u32 {
    20
}

/// Summary of a vote (returned on creation and status queries).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteInfo {
    pub vote_id: String,
    pub room_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub options: Vec<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closes_at: Option<DateTime<Utc>>,
    pub status: VoteStatus,
    /// Number of ballots cast (not WHO voted or WHAT they voted).
    pub votes_cast: usize,
    /// Total eligible voters (room members at vote creation time).
    pub eligible_voters: usize,
    /// Revealed tally for closed votes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tally: Option<Vec<VoteTally>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteStatus {
    Open,
    Closed,
}

/// Revealed vote results, broadcast when vote closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteResultPayload {
    pub vote_id: String,
    pub room_id: String,
    pub title: String,
    pub options: Vec<String>,
    /// Tally: option_index -> count.
    pub tally: Vec<VoteTally>,
    /// Individual ballots revealed.
    pub ballots: Vec<BallotEntry>,
    pub total_votes: usize,
    pub eligible_voters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteTally {
    pub option_index: usize,
    pub option_text: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BallotEntry {
    pub agent_id: String,
    pub agent_name: String,
    pub option_index: usize,
}

// --- Task payloads ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignTaskPayload {
    pub room_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Agent ID to assign the task to. If None, task is unassigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskPayload {
    pub task_id: String,
    /// New status: "pending", "in_progress", "completed", "blocked"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Reassign to a different agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Optional status message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksPayload {
    pub room_id: String,
    /// Filter by status (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// A tracked task within a room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub room_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// --- Presence payloads ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetTypingPayload {
    pub room_id: String,
    /// true = started typing, false = stopped typing
    #[serde(default = "default_typing")]
    pub typing: bool,
}

fn default_typing() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPresencePayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
}

// --- Election payloads ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectLeaderPayload {
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclineElectionPayload {
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPayload {
    pub room_id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

// --- Webhook subscriptions ---

/// Register a webhook to receive HTTP POSTs when matching messages land in `room_id`.
///
/// `secret` is the shared HMAC-SHA256 key used to sign each delivery (Standard
/// Webhooks v1 signature). `since_seq` defaults to the room's current tip on
/// creation if omitted (i.e., only future messages are delivered); pass `0` to
/// backfill the entire room. All filter fields use AND across fields, OR within
/// a single field (e.g., `kinds = ["a","b"]` matches kind==a OR kind==b).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribePayload {
    pub room_id: String,
    pub webhook_url: String,
    pub secret: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_from: Option<String>,
    #[serde(default)]
    pub exclude_thinking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribePayload {
    pub subscription_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnableSubscriptionPayload {
    pub subscription_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSubscriptionsPayload {
    /// Optional filter to one room. Omit to list all subscriptions owned by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
}

/// Returned by `list_subscriptions` / `subscribe`. Mirrors the server's row plus
/// runtime status fields. The `secret` is NEVER returned — only the caller who
/// created it knows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub subscription_id: String,
    pub room_id: String,
    pub webhook_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_from: Option<String>,
    pub exclude_thinking: bool,
    pub since_seq: i64,
    pub last_delivered_seq: i64,
    /// `active` | `failed` | `disabled`
    pub status: String,
    pub failure_count: i64,
    pub created_at: DateTime<Utc>,
}

// --- Thinking pulse ---

/// In-stream "I'm thinking out loud" pulse. Persisted to history like a chat message
/// (with metadata.type = "thinking") so late-joining clients can see the reasoning,
/// but broadcast as a `thinking` event (not `message_received`) so live waiters
/// aren't woken by every pulse, and does NOT advance the room's turn token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingPayload {
    pub room_id: String,
    pub content: String,
}

// --- Turn token events ---

/// Broadcast when the turn-token holder of a room changes.
///
/// `current_turn_holder` is None only when the room is empty. `reason` is one of
/// `"joined"`, `"left"`, `"disconnected"`, `"message_sent"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnChangedPayload {
    pub room_id: String,
    pub current_turn_holder: Option<String>,
    /// Members in join order; the holder is the first connected member in this list
    /// once the token has been initialized.
    pub turn_order: Vec<String>,
    pub reason: String,
}
