use serde::{Deserialize, Serialize};

/// Wire protocol version this build speaks. Sent in `RegisterPayload` and
/// advertised in the register reply. Bump when a `Frame`/payload change is not
/// backward-compatible; raise [`MIN_SUPPORTED_PROTOCOL`] only once the server
/// genuinely drops support for an older shape.
pub const PROTOCOL_VERSION: u32 = 1;

/// Oldest protocol version the server still accepts. A client below this is
/// asked to upgrade rather than failing later with a confusing parse error.
/// A missing version in a register frame is treated as `1` (pre-versioning).
pub const MIN_SUPPORTED_PROTOCOL: u32 = 1;

/// Every message on the wire is a Frame, serialized as a single line of JSON (NDJSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    /// Optional correlation ID for request/response matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// For responses: which request this replies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,

    /// The message type / command.
    #[serde(rename = "type")]
    pub frame_type: FrameType,

    /// Type-specific payload.
    #[serde(default = "default_payload")]
    pub payload: serde_json::Value,
}

fn default_payload() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

impl Frame {
    pub fn ok(reply_to: Option<&str>, payload: serde_json::Value) -> Self {
        Self {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: reply_to.map(String::from),
            frame_type: FrameType::Ok,
            payload,
        }
    }

    pub fn error(reply_to: Option<&str>, error: crate::ErrorPayload) -> Self {
        Self {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: reply_to.map(String::from),
            frame_type: FrameType::Error,
            payload: serde_json::to_value(error).unwrap_or_default(),
        }
    }

    pub fn event(frame_type: FrameType, payload: serde_json::Value) -> Self {
        Self {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: None,
            frame_type,
            payload,
        }
    }

    pub fn pong(reply_to: Option<&str>) -> Self {
        Self {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: reply_to.map(String::from),
            frame_type: FrameType::Pong,
            payload: serde_json::json!({}),
        }
    }

    /// Serialize this frame as a single NDJSON line (with trailing newline).
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }

    /// Parse a frame from a single line of JSON.
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameType {
    // Client -> Server commands
    Register,
    Ping,
    CreateRoom,
    JoinRoom,
    LeaveRoom,
    SendMessage,
    GetHistory,
    ListRooms,
    ListAgents,
    RoomInfo,
    RoomTip,

    // Voting commands (client -> server)
    CreateVote,
    CastVote,
    GetVoteStatus,
    ListVotes,

    // Election commands (client -> server)
    ElectLeader,
    DeclineElection,
    Decision,

    // Task commands (client -> server)
    AssignTask,
    UpdateTask,
    ListTasks,

    // Presence commands (client -> server)
    SetTyping,
    SetPresence,

    // Thinking pulse (client -> server, also broadcast back as event)
    Thinking,

    // Webhook subscriptions (client -> server)
    Subscribe,
    Unsubscribe,
    ListSubscriptions,
    EnableSubscription,

    // Server -> Client responses/events
    Ok,
    Error,
    Pong,
    MessageReceived,
    Mention,
    AgentJoined,
    AgentLeft,
    RoomCreated,
    RoomDestroyed,
    PresenceUpdate,
    HistoryResult,
    RoomList,
    AgentList,
    RoomInfoResult,
    RoomTipResult,

    // Voting events (server -> client)
    VoteCreated,
    VoteResult,

    // Election events (server -> client)
    ElectionStarted,
    LeaderElected,
    LeaderCleared,
    DecisionMade,

    // Task events (server -> client)
    TaskAssigned,
    TaskUpdated,
    TaskList,

    // Presence events (server -> client)
    TypingIndicator,

    // Turn token events (server -> client)
    TurnChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_roundtrip() {
        let frame = Frame {
            id: Some("req-1".into()),
            reply_to: None,
            frame_type: FrameType::SendMessage,
            payload: serde_json::json!({"room_id": "lobby", "content": "hello"}),
        };

        let line = frame.to_line().unwrap();
        let parsed = Frame::from_line(&line).unwrap();
        assert_eq!(parsed.frame_type, FrameType::SendMessage);
        assert_eq!(parsed.id, Some("req-1".into()));
    }

    #[test]
    fn test_frame_type_serialization() {
        let json = serde_json::to_string(&FrameType::MessageReceived).unwrap();
        assert_eq!(json, "\"message_received\"");

        let parsed: FrameType = serde_json::from_str("\"join_room\"").unwrap();
        assert_eq!(parsed, FrameType::JoinRoom);
    }
}
