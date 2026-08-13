use cowchat_core::{AgentInfo, Frame};
use tokio::sync::mpsc;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Internal marker used when a pre-ownership-schema room has a SQL NULL
/// `owner_key`. It is deliberately impossible to authorize, even if a client
/// submits the same bytes as an API key. Newly-created keyless rooms use an
/// empty string in SQLite and are decoded to `None` in the in-memory model.
pub(crate) const LEGACY_UNOWNED_OWNER_KEY: &str = "\0cowchat:legacy-unowned";

/// Decide whether one connection may discover or operate on a room.
///
/// `auth_disabled` is the server-wide `--no-auth` escape hatch. A keyless
/// local connection is represented by an empty `api_key`; unlike global
/// no-auth mode it is deliberately isolated from API-key-owned private rooms.
pub(crate) fn can_access_room_parts(
    visibility: &str,
    owner_key: Option<&str>,
    api_key: &str,
    auth_disabled: bool,
) -> bool {
    if auth_disabled || visibility == "public" {
        return true;
    }

    matches_room_owner(owner_key, api_key, auth_disabled)
}

/// Match the room-owning bearer principal without considering visibility.
/// Public visibility grants discovery/use, never mutation authority.
pub(crate) fn matches_room_owner(
    owner_key: Option<&str>,
    api_key: &str,
    auth_disabled: bool,
) -> bool {
    if owner_key == Some(LEGACY_UNOWNED_OWNER_KEY) {
        return false;
    }
    auth_disabled
        || match owner_key {
            Some(owner_key) => !api_key.is_empty() && owner_key == api_key,
            None => api_key.is_empty(),
        }
}

/// Full room-access check: owner/visibility rules plus invite-issued grants.
/// The grant lookup is an in-memory cache read, so this stays safe on the
/// per-agent broadcast hot path.
pub(crate) fn can_access_room(
    room: &cowchat_core::Room,
    api_key: &str,
    auth_disabled: bool,
    store: &crate::store::Store,
) -> bool {
    can_access_room_parts(
        &room.visibility,
        room.owner_key.as_deref(),
        api_key,
        auth_disabled,
    ) || store.key_has_grant(api_key, &room.room_id)
}

/// Represents a connected agent's server-side state.
pub struct AgentConnection {
    pub info: AgentInfo,
    pub session_id: String,
    pub sender: mpsc::Sender<Frame>,
    pub send_task: JoinHandle<()>,
    pub receive_task: JoinHandle<()>,
    pub disconnect: std::sync::Arc<Notify>,
    /// The API key this agent authenticated with (for room visibility checks).
    pub api_key: String,
}

impl AgentConnection {
    pub fn new(
        info: AgentInfo,
        session_id: String,
        sender: mpsc::Sender<Frame>,
        send_task: JoinHandle<()>,
        receive_task: JoinHandle<()>,
        disconnect: std::sync::Arc<Notify>,
        api_key: String,
    ) -> Self {
        Self {
            info,
            session_id,
            sender,
            send_task,
            receive_task,
            disconnect,
            api_key,
        }
    }
}

impl Drop for AgentConnection {
    fn drop(&mut self) {
        self.disconnect.notify_one();
        self.send_task.abort();
        self.receive_task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::{can_access_room_parts, matches_room_owner};

    #[test]
    fn private_room_access_separates_keyless_and_api_key_owners() {
        assert!(can_access_room_parts("private", None, "", false));
        assert!(!can_access_room_parts(
            "private",
            Some("owner-key"),
            "",
            false
        ));
        assert!(!can_access_room_parts("private", None, "owner-key", false));
        assert!(can_access_room_parts(
            "private",
            Some("owner-key"),
            "owner-key",
            false
        ));
        assert!(can_access_room_parts(
            "private",
            Some("owner-key"),
            "other-key",
            true
        ));
        assert!(can_access_room_parts(
            "public",
            Some("owner-key"),
            "other-key",
            false
        ));
        assert!(!matches_room_owner(Some("owner-key"), "other-key", false));
        assert!(!matches_room_owner(
            Some(super::LEGACY_UNOWNED_OWNER_KEY),
            super::LEGACY_UNOWNED_OWNER_KEY,
            false
        ));
        assert!(!matches_room_owner(
            Some(super::LEGACY_UNOWNED_OWNER_KEY),
            "",
            false
        ));
        assert!(!matches_room_owner(
            Some(super::LEGACY_UNOWNED_OWNER_KEY),
            "",
            true
        ));
    }
}
