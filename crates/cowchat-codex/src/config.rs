use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    #[serde(default)]
    pub cowchat: CowchatConfig,
    #[serde(default)]
    pub codex: CodexConfig,
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
            room_key_env: None,
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
            targets: BTreeMap::from([(
                "reviewer".to_string(),
                TargetConfig {
                    thread_id: "replace-with-codex-thread-id".to_string(),
                    room: "design-review".to_string(),
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

    fn validate(&self) -> Result<(), ConfigError> {
        if self.targets.is_empty() {
            return Err(ConfigError::Invalid(
                "at least one target is required".into(),
            ));
        }
        if self.cowchat.tcp.is_some() == self.cowchat.socket.is_some() {
            return Err(ConfigError::Invalid(
                "configure exactly one of cowchat.tcp or cowchat.socket".into(),
            ));
        }
        if self.codex.request_timeout_seconds == 0 || self.codex.wake_lease_seconds <= 0 {
            return Err(ConfigError::Invalid(
                "Codex request timeout and wake lease must be positive".into(),
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
        assert!(config.codex.app_server_endpoint.starts_with("unix://"));
    }

    #[test]
    fn rejects_ambiguous_cowchat_transport() {
        let mut config = BridgeConfig::example();
        config.cowchat.socket = Some(PathBuf::from("/tmp/cowchat.sock"));
        assert!(config.validate().is_err());
    }
}
