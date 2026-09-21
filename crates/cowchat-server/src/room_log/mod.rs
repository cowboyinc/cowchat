//! Owner-stream log commands and their deterministic room projection.
//!
//! Commands contain authenticated identities and ciphertext, never API keys.
//! The ingress authenticates the caller before constructing a command. The
//! reducer checks log order and business invariants again at application time.
//! Hosted routes remain disabled until ownership and archive gates are wired.
#[cfg(feature = "cbfs-archive")]
pub mod cbfs_archive;
#[cfg(feature = "cbqs")]
pub mod cbqs;
#[cfg(feature = "cbfs-archive")]
pub mod intent;
#[cfg(feature = "cbfs-archive")]
pub mod ownership;
#[cfg(feature = "cbfs-archive")]
pub mod runtime;

use chrono::{DateTime, Utc};
use cowchat_core::ChatMessage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub owner_id: String,
    pub command_id: String,
    pub timestamp: DateTime<Utc>,
    pub body: CommandBody,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandBody {
    CreateRoom {
        room_id: String,
        lane_id: u64,
        name: String,
        created_by: String,
    },
    AppendMessage {
        room_id: String,
        agent_id: String,
        agent_name: String,
        ciphertext: String,
        reply_to: Option<String>,
        metadata: serde_json::Value,
        mentions: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    RoomCreated {
        room_id: String,
        lane_id: u64,
    },
    MessageAppended {
        room_id: String,
        message_id: String,
        sequence: i64,
    },
    Rejected {
        reason: Rejection,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    InvalidIdentity,
    InvalidRoomName,
    RoomExists,
    RoomNameTaken,
    LaneAlreadyAssigned,
    UnknownRoom,
    Plaintext,
    UnknownReply,
    CommandConflict,
    SequenceExhausted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomState {
    pub room_id: String,
    pub lane_id: u64,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub messages: Vec<StoredMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMessage {
    pub message: ChatMessage,
    pub mentions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    digest: [u8; 32],
    outcome: Outcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnerState {
    owner_id: String,
    applied_through: u64,
    rooms: BTreeMap<String, RoomState>,
    receipts: BTreeMap<String, Receipt>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReplayError {
    #[error("owner stream record is not the next sequence")]
    Sequence,
    #[error("record belongs to a different owner")]
    Owner,
    #[error("record is on the wrong room lane")]
    Lane,
    #[error("command cannot be canonically encoded")]
    Encoding,
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn command_digest(command: &Command) -> Result<[u8; 32], ReplayError> {
    let canonical = match &command.body {
        CommandBody::CreateRoom { .. } => serde_json::to_vec(&command.body),
        CommandBody::AppendMessage {
            room_id,
            agent_id,
            ciphertext,
            reply_to,
            metadata,
            mentions,
            ..
        } => serde_json::to_vec(&(room_id, agent_id, ciphertext, reply_to, metadata, mentions)),
    }
    .map_err(|_| ReplayError::Encoding)?;
    let mut hash = Sha256::new();
    hash.update(b"cowchat/owner-command/1\0");
    hash.update(&canonical);
    Ok(hash.finalize().into())
}

impl OwnerState {
    pub fn new(owner_id: String) -> Self {
        Self {
            owner_id,
            applied_through: 0,
            rooms: BTreeMap::new(),
            receipts: BTreeMap::new(),
        }
    }

    pub fn applied_through(&self) -> u64 {
        self.applied_through
    }
    pub fn rooms(&self) -> &BTreeMap<String, RoomState> {
        &self.rooms
    }
    pub fn room(&self, id: &str) -> Option<&RoomState> {
        self.rooms.get(id)
    }

    /// Look up an already applied command without adding another log record.
    /// A changed body under an existing ID returns a conflict, never success.
    pub fn receipt(&self, command: &Command) -> Result<Option<Outcome>, ReplayError> {
        if command.owner_id != self.owner_id {
            return Err(ReplayError::Owner);
        }
        let digest = command_digest(command)?;
        Ok(self.receipts.get(&command.command_id).map(|receipt| {
            if receipt.digest == digest {
                receipt.outcome.clone()
            } else {
                Outcome::Rejected {
                    reason: Rejection::CommandConflict,
                }
            }
        }))
    }

    /// Apply one verified CBQS record. A business rejection consumes log order
    /// and keeps a retry receipt; a corrupt/gapped stream stops replay entirely.
    /// Persist the resulting state and applied position together before using it
    /// as a recovery checkpoint. This reducer itself performs no I/O or effects.
    pub fn apply(
        &mut self,
        sequence: u64,
        lane_id: u64,
        command: &Command,
    ) -> Result<Outcome, ReplayError> {
        if self.applied_through.checked_add(1) != Some(sequence) {
            return Err(ReplayError::Sequence);
        }
        if command.owner_id != self.owner_id {
            return Err(ReplayError::Owner);
        }
        match &command.body {
            CommandBody::CreateRoom { .. } if lane_id != 0 => return Err(ReplayError::Lane),
            CommandBody::AppendMessage { room_id, .. }
                if lane_id == 0
                    || self
                        .rooms
                        .get(room_id)
                        .is_some_and(|room| room.lane_id != lane_id) =>
            {
                return Err(ReplayError::Lane);
            }
            _ => {}
        }
        // A resend may have a different attempted timestamp or display name;
        // only the first accepted presentation/time are retained, like local
        // append receipts. Identity, encrypted bytes and routing stay bound.
        let digest = command_digest(command)?;
        let outcome = if let Some(receipt) = self.receipts.get(&command.command_id) {
            if receipt.digest == digest {
                receipt.outcome.clone()
            } else {
                Outcome::Rejected {
                    reason: Rejection::CommandConflict,
                }
            }
        } else {
            let outcome = if !valid_identity(&command.command_id) {
                Outcome::Rejected {
                    reason: Rejection::InvalidIdentity,
                }
            } else {
                self.apply_new(command)
            };
            self.receipts.insert(
                command.command_id.clone(),
                Receipt {
                    digest,
                    outcome: outcome.clone(),
                },
            );
            outcome
        };
        self.applied_through = sequence;
        Ok(outcome)
    }

    fn apply_new(&mut self, command: &Command) -> Outcome {
        let rejected = |reason| Outcome::Rejected { reason };
        match &command.body {
            CommandBody::CreateRoom {
                room_id,
                lane_id,
                name,
                created_by,
            } => {
                if !valid_identity(room_id) || !valid_identity(created_by) || *lane_id == 0 {
                    return rejected(Rejection::InvalidIdentity);
                }
                let Ok(normalized) = crate::store::normalize_room_name(name) else {
                    return rejected(Rejection::InvalidRoomName);
                };
                if normalized != *name {
                    return rejected(Rejection::InvalidRoomName);
                }
                if self.rooms.contains_key(room_id) {
                    return rejected(Rejection::RoomExists);
                }
                if self.rooms.values().any(|room| room.name == *name) {
                    return rejected(Rejection::RoomNameTaken);
                }
                if self.rooms.values().any(|room| room.lane_id == *lane_id) {
                    return rejected(Rejection::LaneAlreadyAssigned);
                }
                self.rooms.insert(
                    room_id.clone(),
                    RoomState {
                        room_id: room_id.clone(),
                        lane_id: *lane_id,
                        name: name.clone(),
                        created_by: created_by.clone(),
                        created_at: command.timestamp,
                        messages: Vec::new(),
                    },
                );
                Outcome::RoomCreated {
                    room_id: room_id.clone(),
                    lane_id: *lane_id,
                }
            }
            CommandBody::AppendMessage {
                room_id,
                agent_id,
                agent_name,
                ciphertext,
                reply_to,
                metadata,
                mentions,
            } => {
                if !valid_identity(agent_id) || mentions.iter().any(|id| !valid_identity(id)) {
                    return rejected(Rejection::InvalidIdentity);
                }
                let Some(room) = self.rooms.get_mut(room_id) else {
                    return rejected(Rejection::UnknownRoom);
                };
                if !cowchat_core::crypto::is_ciphertext(ciphertext) {
                    return rejected(Rejection::Plaintext);
                }
                if reply_to
                    .as_ref()
                    .is_some_and(|id| !room.messages.iter().any(|m| m.message.message_id == *id))
                {
                    return rejected(Rejection::UnknownReply);
                }
                let Some(sequence) = i64::try_from(room.messages.len())
                    .ok()
                    .and_then(|n| n.checked_add(1))
                else {
                    return rejected(Rejection::SequenceExhausted);
                };
                room.messages.push(StoredMessage {
                    message: ChatMessage {
                        message_id: command.command_id.clone(),
                        room_id: room_id.clone(),
                        agent_id: agent_id.clone(),
                        agent_name: agent_name.clone(),
                        content: ciphertext.clone(),
                        reply_to_message: reply_to.clone(),
                        metadata: metadata.clone(),
                        timestamp: command.timestamp,
                        seq: sequence,
                    },
                    mentions: mentions.clone(),
                });
                Outcome::MessageAppended {
                    room_id: room_id.clone(),
                    message_id: command.command_id.clone(),
                    sequence,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "cbqs-test"))]
mod cbqs_tests;
