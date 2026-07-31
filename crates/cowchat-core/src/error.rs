use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}

impl ErrorPayload {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotRegistered,
    RoomNotFound,
    NotInRoom,
    AlreadyInRoom,
    InvalidPayload,
    AgentIdTaken,
    InternalError,
    RoomNameTaken,
    Unauthorized,
    VoteNotFound,
    VoteClosed,
    AlreadyVoted,
    InvalidOption,
    NotLeader,
    ElectionInProgress,
    NoElectionActive,
    RateLimitAgents,
    RateLimitMessages,
    RateLimitRooms,
    /// Message content exceeds the per-tier byte cap.
    MessageTooLarge,
    /// The client's wire protocol version is outside the server's supported range.
    UnsupportedProtocol,
    AccessDenied,
    TaskNotFound,
    /// Plaintext content was sent to an end-to-end encrypted room.
    PlaintextInEncryptedRoom,
}
