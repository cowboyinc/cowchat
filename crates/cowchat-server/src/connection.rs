use cowchat_core::{AgentInfo, Frame};
use std::collections::HashSet;
use tokio::sync::mpsc;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Represents a connected agent's server-side state.
pub struct AgentConnection {
    pub info: AgentInfo,
    pub session_id: String,
    pub sender: mpsc::Sender<Frame>,
    pub send_task: JoinHandle<()>,
    pub receive_task: JoinHandle<()>,
    pub disconnect: std::sync::Arc<Notify>,
    pub rooms: HashSet<String>,
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
            rooms: HashSet::new(),
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
