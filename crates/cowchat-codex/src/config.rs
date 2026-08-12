use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum WakeHint {
    None,
    #[default]
    Normal,
    Urgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    #[serde(default = "default_state_db")]
    pub state_db: PathBuf,
    pub cowchat: CowchatConfig,
    #[serde(default)]
    pub codex: CodexConfig,
    #[serde(default)]
    pub relay: RelayConfig,
    pub targets: BTreeMap<String, TargetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CowchatConfig {
    #[serde(default = "default_cowchat_tcp")]
    pub tcp: Option<String>,
    #[serde(default)]
    pub socket: Option<PathBuf>,
    #[serde(default = "default_api_key_file")]
    pub api_key_file: PathBuf,
    #[serde(default = "default_agent_name")]
    pub agent_name: String,
    /// Stable, collision-resistant identity for this bridge installation.
    /// Runtime roles append distinct suffixes so the MCP and relay processes
    /// cannot evict one another from Cowchat.
    pub agent_id: String,
    #[serde(default)]
    pub room_key_env: Option<String>,
}

impl Default for CowchatConfig {
    fn default() -> Self {
        Self {
            tcp: default_cowchat_tcp(),
            socket: None,
            api_key_file: default_api_key_file(),
            agent_name: default_agent_name(),
            agent_id: default_agent_id(),
            room_key_env: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRole {
    Mcp,
    Relay,
    Doctor,
}

impl BridgeRole {
    fn suffix(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Relay => "relay",
            Self::Doctor => "doctor",
        }
    }
}

impl CowchatConfig {
    pub fn role_agent_id(&self, role: BridgeRole) -> String {
        format!("{}-{}", self.agent_id, role.suffix())
    }

    pub fn validate_transport(&self) -> Result<(), ConfigError> {
        if self.tcp.is_some() == self.socket.is_some() {
            return Err(ConfigError::Invalid(
                "configure exactly one of cowchat.tcp or cowchat.socket".into(),
            ));
        }
        if let Some(tcp) = &self.tcp {
            if !is_loopback_tcp_address(tcp) {
                return Err(ConfigError::Invalid(format!(
                    "cowchat.tcp {tcp:?} is invalid or not loopback; raw TCP carries bearer credentials and is local-only"
                )));
            }
        }
        if let Some(socket) = &self.socket {
            if !socket.is_absolute() {
                return Err(ConfigError::Invalid(
                    "cowchat.socket must be an absolute path".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    #[serde(default = "default_relay_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_relay_poll_interval_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    #[serde(default = "default_app_server_endpoint")]
    pub app_server_endpoint: String,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_wake_lease_seconds")]
    pub wake_lease_seconds: i64,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            app_server_endpoint: default_app_server_endpoint(),
            bearer_token_env: None,
            request_timeout_seconds: default_request_timeout_seconds(),
            wake_lease_seconds: default_wake_lease_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub thread_id: String,
    pub room: String,
    /// Stable Cowchat identity of the recipient. Required when relay is enabled
    /// so the bridge never wakes a task for its own messages.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Opt this target into the managed durable room-to-task relay.
    #[serde(default)]
    pub relay: bool,
    #[serde(default)]
    pub min_wake_hint: WakeHint,
}

impl BridgeConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|source| ConfigError::Read(path.to_path_buf(), source))?;
        let mut config: Self = serde_json::from_str(&raw).map_err(ConfigError::Parse)?;
        config.expand_paths();
        config.validate()?;
        Ok(config)
    }

    pub fn example() -> Self {
        Self {
            state_db: default_state_db(),
            cowchat: CowchatConfig::default(),
            codex: CodexConfig::default(),
            relay: RelayConfig::default(),
            targets: BTreeMap::from([(
                "reviewer".to_string(),
                TargetConfig {
                    thread_id: "replace-with-codex-thread-id".to_string(),
                    room: "replace-with-canonical-room-uuid".to_string(),
                    agent_id: Some("replace-with-unique-task-agent-id".to_string()),
                    relay: true,
                    min_wake_hint: WakeHint::Normal,
                },
            )]),
        }
    }

    pub fn target(&self, alias: &str) -> Result<&TargetConfig, ConfigError> {
        self.targets
            .get(alias)
            .ok_or_else(|| ConfigError::UnknownTarget(alias.to_string()))
    }

    /// Fingerprint installation-wide identity boundaries. Target-specific
    /// identity is deliberately excluded so changing one target cannot abandon
    /// every other target's durable state.
    pub fn state_scope(&self) -> String {
        let identity = serde_json::json!({
            "schema": 3,
            "cowchat": {
                "tcp": self.cowchat.tcp,
                "socket": self.cowchat.socket,
                "api_key_file": self.cowchat.api_key_file,
                "agent_id": self.cowchat.agent_id,
                "room_key_env": self.cowchat.room_key_env,
            },
            "codex_endpoint": self.codex.app_server_endpoint,
        });
        let digest = Sha256::digest(
            serde_json::to_vec(&identity).expect("bridge config identity is serializable"),
        );
        format!("v3:{digest:x}")
    }

    /// Fingerprint only the semantic identity of one target. Runtime knobs
    /// such as polling, leases, relay enablement, and wake-hint policy do not
    /// change which durable inbox this target owns.
    pub fn target_identity(&self, alias: &str) -> Result<String, ConfigError> {
        let target = self.target(alias)?;
        let identity = serde_json::json!({
            "schema": 1,
            "scope": self.state_scope(),
            "alias": alias,
            "room": target.room,
            "thread_id": target.thread_id,
            "recipient_agent_id": target.agent_id,
        });
        let digest = Sha256::digest(
            serde_json::to_vec(&identity).expect("target config identity is serializable"),
        );
        Ok(format!("target-v1:{digest:x}"))
    }

    fn expand_paths(&mut self) {
        self.state_db = expand_home(&self.state_db);
        self.cowchat.api_key_file = expand_home(&self.cowchat.api_key_file);
        if let Some(socket) = &mut self.cowchat.socket {
            *socket = expand_home(socket);
        }
        if let Some(raw) = self.codex.app_server_endpoint.strip_prefix("unix://") {
            let expanded = expand_home(Path::new(raw));
            self.codex.app_server_endpoint = format!("unix://{}", expanded.display());
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.targets.is_empty() {
            return Err(ConfigError::Invalid(
                "at least one target is required".into(),
            ));
        }
        self.cowchat.validate_transport()?;
        if self.codex.request_timeout_seconds == 0 || self.codex.wake_lease_seconds <= 0 {
            return Err(ConfigError::Invalid(
                "Codex request timeout and wake lease must be positive".into(),
            ));
        }
        if self.relay.poll_interval_ms == 0 {
            return Err(ConfigError::Invalid(
                "relay poll interval must be positive".into(),
            ));
        }
        if self.cowchat.agent_name.trim().is_empty() || self.cowchat.agent_id.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "Cowchat agent_name and agent_id must be non-empty".into(),
            ));
        }
        if self.cowchat.agent_id.len() < 16
            || self.cowchat.agent_id == "cowchat-codex-bridge"
            || self.cowchat.agent_id.starts_with("replace-with-")
        {
            return Err(ConfigError::Invalid(
                "cowchat.agent_id must be a stable collision-resistant installation id (at least 16 characters), not the legacy shared default".into(),
            ));
        }
        for (alias, target) in &self.targets {
            if alias.trim().is_empty()
                || target.thread_id.trim().is_empty()
                || target.room.trim().is_empty()
            {
                return Err(ConfigError::Invalid(format!(
                    "target {alias:?} requires non-empty alias, thread_id, and room"
                )));
            }
            if target.relay
                && target
                    .agent_id
                    .as_deref()
                    .is_none_or(|agent_id| agent_id.trim().is_empty())
            {
                return Err(ConfigError::Invalid(format!(
                    "relay-enabled target {alias:?} requires a non-empty agent_id"
                )));
            }
            if let Some(recipient) = target.agent_id.as_deref() {
                for role in [BridgeRole::Mcp, BridgeRole::Relay, BridgeRole::Doctor] {
                    if recipient == self.cowchat.role_agent_id(role) {
                        return Err(ConfigError::Invalid(format!(
                            "target {alias:?} recipient agent_id collides with the bridge {} role identity",
                            role.suffix()
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(rest) = raw.strip_prefix("~/") else {
        return path.to_path_buf();
    };
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(rest))
        .unwrap_or_else(|| path.to_path_buf())
}

fn default_state_db() -> PathBuf {
    PathBuf::from("~/.cowchat/codex-wake.db")
}

fn default_cowchat_tcp() -> Option<String> {
    Some("127.0.0.1:9229".to_string())
}

fn default_api_key_file() -> PathBuf {
    PathBuf::from("~/.cowchat/auth.key")
}

fn default_agent_name() -> String {
    "cowchat-codex".to_string()
}

fn default_agent_id() -> String {
    format!("cowchat-codex-{}", uuid::Uuid::new_v4())
}

fn is_loopback_tcp_address(address: &str) -> bool {
    if let Ok(address) = address.parse::<SocketAddr>() {
        return address.ip().is_loopback();
    }
    address.rsplit_once(':').is_some_and(|(host, port)| {
        host.eq_ignore_ascii_case("localhost") && port.parse::<u16>().is_ok()
    })
}

fn default_relay_poll_interval_ms() -> u64 {
    1_000
}

fn default_app_server_endpoint() -> String {
    "unix://~/.codex/app-server-control/app-server-control.sock".to_string()
}

fn default_request_timeout_seconds() -> u64 {
    15
}

fn default_wake_lease_seconds() -> i64 {
    300
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config {0}: {1}")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("invalid config JSON: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
    #[error("unknown wake target {0:?}")]
    UnknownTarget(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_is_valid_and_uses_recipient_policy() {
        let config = BridgeConfig::example();
        config.validate().unwrap();
        assert_eq!(config.targets["reviewer"].min_wake_hint, WakeHint::Normal);
        assert!(config.targets["reviewer"].relay);
        assert!(config.targets["reviewer"].agent_id.is_some());
        assert!(config.codex.app_server_endpoint.starts_with("unix://"));
    }

    #[test]
    fn rejects_ambiguous_cowchat_transport() {
        let mut config = BridgeConfig::example();
        config.cowchat.socket = Some(PathBuf::from("/tmp/cowchat.sock"));
        assert!(config.validate().is_err());
    }

    #[test]
    fn relay_requires_recipient_agent_id() {
        let mut config = BridgeConfig::example();
        config.targets.get_mut("reviewer").unwrap().agent_id = None;
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_shared_identity_remote_tcp_and_recipient_collision() {
        let mut config = BridgeConfig::example();
        config.cowchat.agent_id = "cowchat-codex-bridge".into();
        assert!(config.validate().is_err());

        config.cowchat.agent_id = "collision-resistant-installation-id".into();
        config.cowchat.tcp = Some("203.0.113.10:9229".into());
        assert!(config.validate().is_err());

        config.cowchat.tcp = Some("127.0.0.1:9229".into());
        config.targets.get_mut("reviewer").unwrap().agent_id =
            Some(config.cowchat.role_agent_id(BridgeRole::Relay));
        assert!(config.validate().is_err());
    }

    #[test]
    fn state_scope_is_common_while_target_identity_is_target_local() {
        let config = BridgeConfig::example();
        let original = config.state_scope();
        let original_target = config.target_identity("reviewer").unwrap();
        let mut changed = config.clone();
        changed.targets.get_mut("reviewer").unwrap().thread_id = "another-thread".into();
        assert_eq!(original, changed.state_scope());
        assert_ne!(
            original_target,
            changed.target_identity("reviewer").unwrap()
        );
        changed = config.clone();
        changed.cowchat.tcp = Some("localhost:9230".into());
        assert_ne!(original, changed.state_scope());

        changed = config.clone();
        changed.targets.insert(
            "another".into(),
            TargetConfig {
                thread_id: "other-thread".into(),
                room: "other-room".into(),
                agent_id: Some("other-agent".into()),
                relay: true,
                min_wake_hint: WakeHint::Urgent,
            },
        );
        assert_eq!(original, changed.state_scope());
        assert_eq!(
            original_target,
            changed.target_identity("reviewer").unwrap()
        );
    }

    #[test]
    fn serialized_config_requires_an_explicit_stable_bridge_identity() {
        let value = serde_json::json!({
            "cowchat": {"tcp": "127.0.0.1:9229"},
            "targets": {
                "reviewer": {
                    "thread_id": "thread",
                    "room": "room"
                }
            }
        });
        assert!(serde_json::from_value::<BridgeConfig>(value).is_err());
        assert_ne!(
            BridgeConfig::example()
                .cowchat
                .role_agent_id(BridgeRole::Mcp),
            BridgeConfig::example()
                .cowchat
                .role_agent_id(BridgeRole::Mcp)
        );
    }
}
