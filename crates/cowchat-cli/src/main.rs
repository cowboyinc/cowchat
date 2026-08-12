use clap::{Parser, Subcommand, ValueEnum};
use cowchat_client::{ClientError, CowchatClient};
use cowchat_core::{ChatMessage, ErrorCode, FrameType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};

mod lantern;

/// `metadata.kind` marking the last message of a conversation. `send --end` sets
/// it; `wait` exits 3 on receiving one so a reply-then-wait loop terminates.
const KIND_CONVERSATION_END: &str = "conversation_end";

fn render_export(
    messages: &[ChatMessage],
    format: ExportFormat,
    include_thinking: bool,
    room_label: &str,
) -> String {
    let mut out = String::new();
    let filtered: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| {
            let kind = m.metadata.get("type").and_then(|v| v.as_str());
            if kind == Some("system") {
                return false; // never include system rows in exports
            }
            if !include_thinking && kind == Some("thinking") {
                return false;
            }
            true
        })
        .collect();

    match format {
        ExportFormat::Md => {
            out.push_str(&format!("# Room: {}\n\n", room_label));
            if let (Some(first), Some(last)) = (filtered.first(), filtered.last()) {
                out.push_str(&format!(
                    "_{} entries from seq {} to {} ({} → {})_\n\n",
                    filtered.len(),
                    first.seq,
                    last.seq,
                    first.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                    last.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                ));
            }
            for m in &filtered {
                let kind = m.metadata.get("type").and_then(|v| v.as_str());
                let header = match kind {
                    Some("thinking") => format!(
                        "### 💭 {} · seq {} · {}",
                        m.agent_name,
                        m.seq,
                        m.timestamp.format("%H:%M:%S")
                    ),
                    _ => format!(
                        "### {} · seq {} · {}",
                        m.agent_name,
                        m.seq,
                        m.timestamp.format("%H:%M:%S")
                    ),
                };
                out.push_str(&header);
                out.push_str("\n\n");
                out.push_str(&m.content);
                if !m.content.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }
        ExportFormat::Json => {
            for m in &filtered {
                out.push_str(&serde_json::to_string(m).unwrap_or_default());
                out.push('\n');
            }
        }
        ExportFormat::Txt => {
            for m in &filtered {
                let kind = m.metadata.get("type").and_then(|v| v.as_str());
                let tag = if kind == Some("thinking") {
                    "(thinking) "
                } else {
                    ""
                };
                out.push_str(&format!(
                    "[{}] #{} {}{}: {}\n",
                    m.timestamp.format("%H:%M:%S"),
                    m.seq,
                    tag,
                    m.agent_name,
                    m.content
                ));
                out.push('\n');
            }
        }
    }
    out
}

fn format_message(msg: &ChatMessage) -> String {
    let ts = msg.timestamp.format("%H:%M:%S");
    let is_system = msg.metadata.get("type").and_then(|v| v.as_str()) == Some("system");
    if is_system {
        format!(
            "[{}] #{} * {} {} *",
            ts, msg.seq, msg.agent_name, msg.content
        )
    } else {
        format!("[{}] #{} {}: {}", ts, msg.seq, msg.agent_name, msg.content)
    }
}

#[derive(Parser)]
#[command(
    name = "cowchat",
    version,
    about = "Cowchat - Agent-to-agent chat infrastructure"
)]
struct Cli {
    /// Unix socket path to connect to
    #[arg(long, global = true, default_value = default_socket_path())]
    socket: PathBuf,

    /// Use TCP instead of Unix socket
    #[arg(long, global = true)]
    tcp: Option<String>,

    /// Connect over WebSocket to a remote server, e.g.
    /// wss://your-server.example/ws. Takes precedence over --tcp/--socket.
    #[arg(long, global = true)]
    url: Option<String>,

    /// API key for authenticated remote servers (local connections are keyless)
    #[arg(long, global = true)]
    key: Option<String>,

    /// Pre-shared key for end-to-end encrypted rooms. Overrides the
    /// COWCHAT_ROOM_KEY environment variable.
    /// Content is encrypted before send and decrypted after receive, per-room.
    #[arg(long, global = true)]
    room_key: Option<String>,

    /// Agent name for this CLI session
    #[arg(long, global = true, default_value = "cli")]
    name: String,

    /// Stable agent id for this session. Pass a consistent, task-unique value
    /// across calls so the server treats separate send/wait invocations as ONE
    /// agent. COWCHAT_AGENT_ID is used when this flag is omitted. Agent-facing
    /// commands fail instead of silently creating a random identity when neither
    /// is set.
    #[arg(long, global = true)]
    agent_id: Option<String>,

    /// Explicitly import an old unscoped integer cursor. This asserts that the
    /// cursor belongs to this exact endpoint, room, and agent; it is rewritten
    /// as a scoped cursor after successful validation.
    #[arg(long, global = true)]
    import_legacy_cursor: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a message to a room
    Send {
        /// Room ID or name
        room: String,
        /// Message content
        message: String,
        /// Reply to a specific message ID
        #[arg(long)]
        reply_to: Option<String>,
        /// Tag the message with a `kind` (stored in `metadata.kind`). Free-form,
        /// but conventions: `review_request`, `verdict`, `checkpoint`, `fyi`.
        /// Peers can filter on these via `wait --only-kind` / `history --kind`.
        #[arg(long)]
        kind: Option<String>,
        /// Cursor file shared with the returning wait loop. If the file does
        /// not exist, initialize it to zero for at-least-once delivery. Prefer
        /// initializing it precisely with `history --cursor-file` during catch-up.
        #[arg(long)]
        cursor_file: Option<PathBuf>,
        /// End the conversation: tags the message `kind=conversation_end`. A peer
        /// running `wait` surfaces this message, then exits 3 so its reply-then-wait
        /// loop terminates cleanly instead of blocking for another turn.
        #[arg(long, conflicts_with = "kind")]
        end: bool,
    },

    /// Post a "thinking out loud" pulse to a room (persisted to history,
    /// broadcast as a `thinking` event, does NOT advance the turn token,
    /// does NOT wake peers' `wait`).
    Thinking {
        /// Room ID or name
        room: String,
        /// Thought / status content (keep it short)
        content: String,
    },

    /// Room management
    Rooms {
        #[command(subcommand)]
        action: RoomAction,
    },

    /// List connected agents
    Agents {
        /// Filter by room ID
        #[arg(long)]
        room: Option<String>,
    },

    /// View message history
    History {
        /// Room ID
        room: String,
        /// Number of messages. With --cursor-file this is the page size; catch-up
        /// continues through the room tip captured at command start.
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Stream new messages (like tail -f)
        #[arg(long)]
        follow: bool,
        /// Only return messages after this message ID
        #[arg(long)]
        since: Option<String>,
        /// Only return messages with seq strictly greater than this value
        #[arg(long)]
        since_seq: Option<i64>,
        /// Only return messages tagged with this `metadata.kind`
        /// (e.g. `--kind verdict`).
        #[arg(long)]
        kind: Option<String>,
        /// Write to file instead of stdout
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// After successfully evaluating the contiguous catch-up through its
        /// captured tip, persist that tip as the floor for subsequent send/wait
        /// commands. Display filters do not pin progress. Use the same
        /// per-server/room/identity path throughout the conversation.
        #[arg(long, conflicts_with = "follow")]
        cursor_file: Option<PathBuf>,
    },

    /// Wait for a new message in a room (blocks until one arrives)
    Wait {
        /// Room ID or name
        room: String,
        /// Timeout in seconds (0 = wait forever). In --loop mode this is the
        /// per-iteration budget; outer wall-clock is unbounded.
        #[arg(long, default_value = "60")]
        timeout: u64,
        /// Output as JSON. Default. Pass --text for human-readable output.
        #[arg(long, conflicts_with = "text")]
        json: bool,
        /// Output as human-readable text instead of JSON.
        #[arg(long)]
        text: bool,
        /// Also catch up: return the oldest chat message with seq > this value
        /// if one already exists in history, else block for a new message.
        /// Use this to safely resume after a prior wait — pass the seq of the
        /// last message you saw. Accepts an integer, or `tip`/`auto` to resolve
        /// to the room's current tip on start.
        #[arg(long)]
        since_seq: Option<String>,
        /// Stay in wait indefinitely: re-poll on internal timeout and reconnect
        /// transport failures with bounded backoff (tracking the bookmark) until
        /// a real chat message arrives. Pairs with --since-seq so messages that
        /// land between iterations are never missed. With this flag the single
        /// CLI invocation replaces the "re-run wait on timeout" discipline — the
        /// agent makes one call and gets one message back.
        #[arg(long = "loop")]
        loop_: bool,
        /// Keep streaming messages until interrupted or a conversation_end is
        /// received. Uses a durable cursor, reconnects with backoff, and emits
        /// every matching message instead of returning after the first one.
        #[arg(long)]
        follow: bool,
        /// Bound the total wall-clock of a `--loop` wait (seconds). On expiry the
        /// command exits 2 (distinct from 0=message, 1=error) and prints the seq to
        /// resume from, so a stalled turn returns control instead of hanging forever.
        /// 0 = unbounded (default). Without `--loop`, `--timeout` already bounds the
        /// single wait, so this has no effect there.
        #[arg(long, default_value = "0")]
        idle_timeout: u64,
        /// Seconds between liveness heartbeats printed to stderr while blocked.
        /// Tool wrappers that kill silent processes see this and let the wait
        /// continue. 0 disables. Default 30.
        #[arg(long, default_value = "30")]
        heartbeat_secs: u64,
        /// Only wake on messages from this `--name` (peer filter).
        #[arg(long)]
        only_from: Option<String>,
        /// Skip messages from this `--name` (in addition to your own).
        #[arg(long)]
        not_from: Option<String>,
        /// Only wake on messages tagged with this `metadata.kind`.
        /// E.g. `--only-kind review_request`.
        #[arg(long)]
        only_kind: Option<String>,
        /// Print peer `thinking` pulses to stderr while blocked, for live
        /// visibility during long runs. Does NOT wake the wait — it still only
        /// returns on a real chat message, preserving turn-taking. Content is
        /// decrypted if a room key is set.
        #[arg(long)]
        show_thinking: bool,
        /// Write result to file instead of stdout. Useful when tool wrappers
        /// truncate large stdout payloads.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Wake on the next message, then emit EVERY unread message through the
        /// room's current tip (one JSON object per line), not just the one that
        /// woke the wait. Drain before composing so a correction that landed
        /// while you were thinking isn't answered a turn late. The cursor advances
        /// to the last message in the batch. Requires persisted room history;
        /// ephemeral rooms fail closed rather than checkpointing an unseen gap.
        #[arg(long)]
        drain: bool,
        /// Persist the highest processed seq to this file and read it back as the
        /// floor on the next run — so you run the SAME command each turn and the
        /// read cursor only ever advances to messages you actually received.
        /// Takes precedence over --since-seq (which then just seeds the first
        /// run, before the file exists). Initialize the same path with the one-time
        /// history catch-up, and pass it to every send. If send sees a missing
        /// cursor it starts at zero rather than guessing that the current tip was
        /// read. Corrupt or unwritable cursor files fail closed.
        #[arg(long)]
        cursor_file: Option<PathBuf>,
    },

    /// Monitor events in real-time
    Monitor {
        /// Filter to a specific room
        #[arg(long)]
        room: Option<String>,
        /// Output raw JSON frames
        #[arg(long)]
        json: bool,
    },

    /// Interactive persistent session for room coordination
    Shell {
        /// Optional room ID or name to join on start
        #[arg(long)]
        room: Option<String>,
    },

    /// Show server status
    Status,

    /// Generate a random end-to-end room key (for COWCHAT_ROOM_KEY). Print it
    /// once, then set the SAME value on every agent in the group so they can
    /// read each other's encrypted messages. No server connection needed.
    Keygen,

    /// Webhook subscriptions — register an HTTP endpoint to be POSTed when
    /// matching messages land in a room. Lets external automations react to
    /// events without holding a long-running `wait --loop` open.
    Sub {
        #[command(subcommand)]
        action: SubAction,
    },

    /// Export a room's history as markdown (or json/text).
    Export {
        /// Room ID or exact name
        room: String,
        /// Output format
        #[arg(long, default_value = "md")]
        format: ExportFormat,
        /// Only include messages with seq > this value
        #[arg(long)]
        since_seq: Option<i64>,
        /// Maximum messages to include (default: all)
        #[arg(long)]
        limit: Option<u32>,
        /// Include `thinking` pulses in the export. Off by default — most
        /// archives want only the chat narrative.
        #[arg(long)]
        include_thinking: bool,
        /// Write to file instead of stdout
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Voting commands
    Vote {
        #[command(subcommand)]
        action: VoteAction,
    },

    /// Leader election commands
    Election {
        #[command(subcommand)]
        action: ElectionAction,
    },

    /// Set agent presence status (idle, waiting, working, thinking)
    Presence {
        /// Status: idle, waiting, working, or thinking
        status: String,
        /// Human-readable detail, e.g. "reviewing section 3"
        #[arg(long)]
        detail: Option<String>,
        /// Progress percentage (0-100)
        #[arg(long)]
        progress: Option<u8>,
    },

    /// LANTERN: an optional structured-reasoning overlay (HELLO + falsifiable
    /// claims, challenges, resolutions, synthesis). Carried inside message
    /// content — no server changes; state is reconstructed from history. Use it
    /// when a conversation is contested, high-stakes, or state-changing.
    Lantern {
        #[command(subcommand)]
        action: LanternAction,
    },
}

#[derive(Subcommand)]
enum LanternAction {
    /// Send a HELLO provenance preamble (identity, role, capability claims —
    /// all self-attested; advertises, does NOT grant, permissions).
    Hello {
        /// Room ID or name
        room: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        role: Option<String>,
        /// Repeatable capability as `name` or `name=falsifiable_by`.
        #[arg(long = "capability")]
        capabilities: Vec<String>,
    },
    /// Open a thread with a question.
    Probe {
        room: String,
        question: String,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Make a falsifiable claim (opens a thread). `--falsifiable-by` is required.
    Assert {
        room: String,
        #[arg(long)]
        claim: String,
        #[arg(long)]
        confidence: Option<f64>,
        #[arg(long = "falsifiable-by")]
        falsifiable_by: String,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Counter an ASSERT/CHALLENGE in a thread. Must stake confidence + a test.
    Challenge {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long = "target-seq")]
        target_seq: i64,
        #[arg(long = "counter-claim")]
        counter_claim: String,
        #[arg(long)]
        confidence: f64,
        #[arg(long)]
        test: String,
    },
    /// Record the observation that settles a branch (basis: tool/artifact/human/consensus/stale).
    Resolve {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long)]
        observation: String,
        #[arg(long)]
        basis: String,
    },
    /// Commit synthesis + shared-state delta into a thread (the commit point).
    Fuse {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long)]
        synthesis: String,
        /// Path to a JSON file holding the shared_state_delta object.
        #[arg(long = "state-delta")]
        state_delta: Option<PathBuf>,
        /// Preserve an intentional split rather than committing one model.
        #[arg(long)]
        split: bool,
        /// Repeatable calibration verdict `<seq>=<true|false>`: did that staked
        /// claim hold? Only scored when the thread's RESOLVE basis is tool/artifact/human.
        #[arg(long = "outcome")]
        outcomes: Vec<String>,
    },
    /// Reconcile shared state without prose (a state hash + JSON diff file).
    Sync {
        room: String,
        #[arg(long)]
        thread: Option<i64>,
        #[arg(long = "state-hash")]
        state_hash: Option<String>,
        #[arg(long = "diff")]
        diff: Option<PathBuf>,
    },
    /// Introduce a scarce, orthogonal idea (side channel; answer with harvest/bury).
    Spark {
        room: String,
        #[arg(long)]
        seed: String,
        #[arg(long = "why-now")]
        why_now: String,
        #[arg(long = "smallest-test")]
        smallest_test: String,
    },
    /// Accept a SPARK into the working set.
    Harvest {
        room: String,
        #[arg(long = "spark-seq")]
        spark_seq: i64,
        #[arg(long)]
        becomes: Option<String>,
    },
    /// Decline a SPARK in one sentence.
    Bury {
        room: String,
        #[arg(long = "spark-seq")]
        spark_seq: i64,
        #[arg(long)]
        reason: String,
    },
    /// List threads in a room (id, state, headline).
    Threads { room: String },
    /// Show every message in one thread, in order.
    Show { room: String, thread: i64 },
    /// Show the committed shared-state deltas (the agreed model).
    State { room: String },
    /// Show per-agent calibration loss (lower is better; diagnostic only).
    Calibration { room: String },
    /// Validate a LANTERN envelope from a file (or `-` for stdin).
    Validate { path: String },
}

#[derive(Subcommand)]
enum RoomAction {
    /// List all rooms
    List {
        /// Filter by parent room ID
        #[arg(long)]
        parent: Option<String>,
    },
    /// Create a new room
    Create {
        /// Room name
        #[arg(value_name = "NAME")]
        room_name: String,
        /// Room description
        #[arg(long)]
        description: Option<String>,
        /// Parent room ID
        #[arg(long)]
        parent: Option<String>,
        /// Create as ephemeral (auto-deleted when empty)
        #[arg(long)]
        ephemeral: bool,
        /// Create as public: visible and joinable by any API key on the server.
        /// Default is private (only your API-key or keyless-local boundary can
        /// resolve it by name) — use this so other principals can find the room.
        #[arg(long)]
        public: bool,
        /// Create as end-to-end encrypted. Members must share a room key
        /// (--room-key or $COWCHAT_ROOM_KEY); the server stores only
        /// ciphertext and rejects plaintext sends to this room.
        #[arg(long)]
        encrypted: bool,
    },
    /// Get room info
    Info {
        /// Room ID
        room_id: String,
    },
    /// Get the latest seq for a room (the room's "tip")
    Tip {
        /// Room ID or exact name
        room: String,
    },
    /// Rename a room using its owning principal and recorded creator ID.
    Rename {
        /// Room ID or exact name
        room: String,
        /// New room name
        #[arg(value_name = "NAME")]
        new_name: String,
    },
    /// Irreversibly remove a room from Cowchat using its owning principal and creator ID.
    Destroy {
        /// Room ID or exact name
        room: String,
        /// Confirm the irreversible deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum VoteAction {
    /// Create a sealed-ballot vote in a room
    Create {
        /// Room ID or exact name
        room: String,
        /// Vote title / question
        title: String,
        /// Vote options (at least 2)
        #[arg(long, num_args = 2.., required = true)]
        options: Vec<String>,
        /// Optional description
        #[arg(long)]
        description: Option<String>,
        /// Deadline in seconds
        #[arg(long)]
        duration: Option<u64>,
    },
    /// Cast a ballot on an active vote
    Cast {
        /// Vote ID
        vote_id: String,
        /// Option index (0-based)
        option: usize,
    },
    /// Check status of a vote
    Status {
        /// Vote ID
        vote_id: String,
    },
    /// List recent votes in a room
    History {
        /// Room ID or exact room name
        room: String,
        /// Maximum number of votes to return
        #[arg(long, default_value = "20")]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum ElectionAction {
    /// Start a leader election in a room
    Start {
        /// Room ID or exact name
        room: String,
    },
    /// Decline an active election
    Decline {
        /// Room ID or exact name
        room: String,
    },
    /// Issue a decision as room leader
    Decide {
        /// Room ID or exact name
        room: String,
        /// Decision content
        content: String,
    },
}

#[derive(Subcommand)]
enum SubAction {
    /// Create a new webhook subscription on a room.
    Create {
        /// Room ID or exact name
        room: String,
        /// Webhook URL (http or https). The server will POST messages here.
        #[arg(long)]
        url: String,
        /// HMAC-SHA256 shared secret used to sign each delivery (Standard
        /// Webhooks v1 signature). Keep it private — receivers verify with it.
        #[arg(long)]
        secret: String,
        /// Restrict to specific `--kind` values on the message (comma-separated).
        #[arg(long, value_delimiter = ',')]
        kinds: Vec<String>,
        /// Only deliver messages from this `--name`.
        #[arg(long)]
        only_from: Option<String>,
        /// Skip messages from this `--name`.
        #[arg(long)]
        not_from: Option<String>,
        /// Don't deliver `thinking` pulses (only real chat).
        #[arg(long)]
        exclude_thinking: bool,
        /// Start cursor. Integer, or `tip`/`auto` (default) for "only future
        /// messages", or `0` to backfill the entire room.
        #[arg(long, default_value = "tip")]
        since_seq: String,
    },
    /// List subscriptions owned by your API key.
    List {
        /// Optionally filter to one room.
        #[arg(long)]
        room: Option<String>,
    },
    /// Delete a subscription.
    Delete { subscription_id: String },
    /// Re-enable a `failed` subscription. Replays the backlog past the cursor.
    Enable { subscription_id: String },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ExportFormat {
    /// Markdown — chat-style rendering with timestamps + agent labels.
    Md,
    /// One JSON object per message, newline-delimited.
    Json,
    /// Plain text — human-readable, no markup.
    Txt,
}

fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat"))
        .unwrap_or_else(|| PathBuf::from(".cowchat"))
}

fn default_socket_path() -> &'static str {
    Box::leak(
        default_data_dir()
            .join("cowchat.sock")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    )
}

fn default_key_path() -> PathBuf {
    default_data_dir().join("auth.key")
}

fn load_key(key_arg: &Option<String>) -> String {
    if let Some(key) = key_arg {
        return key.clone();
    }
    let key_path = default_key_path();
    std::fs::read_to_string(key_path)
        .map(|key| key.trim().to_string())
        .unwrap_or_default()
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn resolve_agent_id(cli: &Cli) -> Option<String> {
    cli.agent_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| env_non_empty("COWCHAT_AGENT_ID"))
}

fn command_requires_stable_agent_id(command: &Commands) -> bool {
    match command {
        Commands::Send { .. }
        | Commands::Thinking { .. }
        | Commands::Wait { .. }
        | Commands::Shell { .. }
        | Commands::Presence { .. }
        | Commands::Election { .. } => true,
        Commands::History {
            follow,
            cursor_file,
            ..
        } => *follow || cursor_file.is_some(),
        Commands::Rooms { action } => matches!(
            action,
            RoomAction::Create { .. } | RoomAction::Rename { .. } | RoomAction::Destroy { .. }
        ),
        Commands::Vote { action } => {
            matches!(action, VoteAction::Create { .. } | VoteAction::Cast { .. })
        }
        Commands::Lantern { action } => !matches!(
            action,
            LanternAction::Threads { .. }
                | LanternAction::Show { .. }
                | LanternAction::State { .. }
                | LanternAction::Calibration { .. }
                | LanternAction::Validate { .. }
        ),
        _ => false,
    }
}

fn command_opens_connection(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Keygen
            | Commands::Lantern {
                action: LanternAction::Validate { .. }
            }
    )
}

pub(crate) fn resolve_room_key(flag: Option<String>) -> Option<String> {
    flag.filter(|v| !v.is_empty())
        .or_else(|| env_non_empty("COWCHAT_ROOM_KEY"))
}

/// Resolve the end-to-end room secret. Returns None when no flag or supported
/// environment variable is set (the client then sends/receives plaintext).
fn resolve_room_secret(cli: &Cli) -> Option<Vec<u8>> {
    resolve_room_key(cli.room_key.clone()).map(String::into_bytes)
}

/// Decrypt a raw frame `content` field for display. Used on pushed events
/// (monitor/shell/follow) where the client API hasn't already decrypted. Falls
/// back to the original string when there's no key or it isn't a ciphertext blob.
fn decrypt_field(secret: Option<&[u8]>, room_id: &str, content: &str) -> String {
    match secret {
        Some(s) if cowchat_core::crypto::is_ciphertext(content) => {
            cowchat_core::crypto::decrypt(s, room_id, content)
                .unwrap_or_else(|_| content.to_string())
        }
        _ => content.to_string(),
    }
}

async fn connect(cli: &Cli) -> Result<CowchatClient, Box<dyn std::error::Error>> {
    let key = load_key(&cli.key);

    let agent_id = resolve_agent_id(cli);
    let mut client = if let Some(url) = &cli.url {
        CowchatClient::connect_ws(url, &key, &cli.name, agent_id.as_deref(), vec![]).await?
    } else if let Some(addr) = &cli.tcp {
        CowchatClient::connect_tcp(addr, &key, &cli.name, agent_id.as_deref(), vec![]).await?
    } else {
        CowchatClient::connect_uds(&cli.socket, &key, &cli.name, agent_id.as_deref(), vec![])
            .await?
    };
    if let Some(secret) = resolve_room_secret(cli) {
        client.set_room_secret(&secret);
    }
    Ok(client)
}

async fn resolve_room_id(
    client: &CowchatClient,
    room: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // Fast path: already a room ID.
    match client.room_info(room).await {
        Ok(_) => return Ok(room.to_string()),
        Err(ClientError::Server {
            code: ErrorCode::RoomNotFound,
            ..
        }) => {}
        Err(e) => return Err(Box::new(e)),
    }

    // Fallback: resolve by exact room name.
    let rooms = client.list_rooms(None).await?;
    let matches: Vec<_> = rooms.into_iter().filter(|r| r.name == room).collect();

    match matches.as_slice() {
        [single] => Ok(single.room_id.clone()),
        [] => Err(format!("Room '{room}' not found (expected ID or exact name)").into()),
        _ => Err(format!("Room name '{room}' is ambiguous; use the room ID").into()),
    }
}

const CURSOR_VERSION: u32 = 2;
static CURSOR_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CursorScope {
    endpoint: String,
    room_id: String,
    agent_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorState {
    version: u32,
    endpoint: String,
    room_id: String,
    agent_id: String,
    seq: i64,
}

#[derive(Clone, Copy, Debug)]
struct LoadedCursor {
    seq: i64,
    needs_upgrade: bool,
    unscoped_legacy: bool,
}

impl CursorState {
    fn new(scope: &CursorScope, seq: i64) -> Self {
        Self {
            version: CURSOR_VERSION,
            endpoint: scope.endpoint.clone(),
            room_id: scope.room_id.clone(),
            agent_id: scope.agent_id.clone(),
            seq,
        }
    }
}

fn cursor_scope(cli: &Cli, room_id: &str) -> Result<CursorScope, Box<dyn std::error::Error>> {
    let endpoint_descriptor = if let Some(url) = &cli.url {
        format!("url:{url}")
    } else if let Some(addr) = &cli.tcp {
        format!("tcp:{addr}")
    } else {
        format!("uds:{}", std::path::absolute(&cli.socket)?.display())
    };
    let endpoint = endpoint_fingerprint(&endpoint_descriptor);
    let agent_id = resolve_agent_id(cli)
        .ok_or("cursor files require --agent-id <UNIQUE_TASK_AGENT_ID> or COWCHAT_AGENT_ID")?;
    Ok(CursorScope {
        endpoint,
        room_id: room_id.to_string(),
        agent_id,
    })
}

fn endpoint_fingerprint(endpoint_descriptor: &str) -> String {
    let digest = Sha256::digest(endpoint_descriptor.as_bytes());
    format!("sha256:{digest:x}")
}

fn cursor_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

fn validate_sequence_floor(label: &str, seq: i64, room_tip: i64) -> std::io::Result<()> {
    if seq < 0 {
        return Err(cursor_error(format!("{label} has negative seq {seq}")));
    }
    if seq > room_tip {
        return Err(cursor_error(format!(
            "{label} is ahead of room tip ({seq} > {room_tip}); the room may have been reset or this sequence belongs to another room"
        )));
    }
    Ok(())
}

fn validate_cursor_seq(path: &Path, seq: i64, room_tip: i64) -> std::io::Result<()> {
    validate_sequence_floor(&format!("cursor file {}", path.display()), seq, room_tip)
}

fn existing_cursor_seq_for_write(
    path: &Path,
    expected_scope: &CursorScope,
    allow_unscoped_legacy: bool,
) -> std::io::Result<Option<i64>> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("refusing to replace symlink cursor file {}", path.display()),
            ));
        }
        if !metadata.file_type().is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("cursor path {} is not a regular file", path.display()),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("refusing hard-linked cursor file {}", path.display()),
                ));
            }
        }
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if let Ok(seq) = raw.trim().parse::<i64>() {
        if !allow_unscoped_legacy {
            return Err(cursor_error(format!(
                "refusing unscoped legacy integer cursor {}; rerun with --import-legacy-cursor only after verifying its endpoint, room, and agent",
                path.display()
            )));
        }
        if seq < 0 {
            return Err(cursor_error(format!(
                "cursor file {} has negative seq {seq}",
                path.display()
            )));
        }
        return Ok(Some(seq));
    }
    let state: CursorState = serde_json::from_str(&raw).map_err(|error| {
        cursor_error(format!(
            "invalid cursor file {} while checkpointing: {error}",
            path.display()
        ))
    })?;
    if !cursor_state_matches_scope(&state, expected_scope) || state.seq < 0 {
        return Err(cursor_error(format!(
            "cursor file {} changed version, scope, or sequence while checkpointing",
            path.display()
        )));
    }
    Ok(Some(state.seq))
}

fn cursor_state_matches_scope(state: &CursorState, expected_scope: &CursorScope) -> bool {
    let endpoint = match state.version {
        CURSOR_VERSION => state.endpoint.clone(),
        1 => endpoint_fingerprint(&state.endpoint),
        _ => return false,
    };
    CursorScope {
        endpoint,
        room_id: state.room_id.clone(),
        agent_id: state.agent_id.clone(),
    } == *expected_scope
}

fn cursor_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_cursor_atomic(path: &Path, scope: &CursorScope, seq: i64) -> std::io::Result<()> {
    write_cursor_atomic_inner(path, scope, seq, false)
}

fn import_legacy_cursor_atomic(path: &Path, scope: &CursorScope, seq: i64) -> std::io::Result<()> {
    write_cursor_atomic_inner(path, scope, seq, true)
}

fn write_cursor_atomic_inner(
    path: &Path,
    scope: &CursorScope,
    seq: i64,
    allow_unscoped_legacy: bool,
) -> std::io::Result<()> {
    if seq < 0 {
        return Err(cursor_error(format!(
            "refusing to persist negative cursor {seq}"
        )));
    }
    let encoded = serde_json::to_vec(&CursorState::new(scope, seq))
        .map_err(|error| cursor_error(format!("failed to serialize cursor: {error}")))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cursor");
    let parent = cursor_parent(path);
    // A dedicated cursor inode cannot safely carry a lock because atomic rename
    // replaces that inode. Lock the containing directory instead, then re-read
    // the durable value under the lock so concurrent processes cannot regress it.
    let directory = std::fs::File::open(parent)?;
    directory.lock()?;
    if existing_cursor_seq_for_write(path, scope, allow_unscoped_legacy)?
        .is_some_and(|current| current > seq)
    {
        return Err(cursor_error(format!(
            "refusing to move cursor {} backwards to {seq}",
            path.display()
        )));
    }

    for _ in 0..128 {
        let counter = CURSOR_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(
            ".{file_name}.{}.{}.{}.tmp",
            std::process::id(),
            nanos,
            counter
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            file.write_all(&encoded)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            directory.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        return result;
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "could not allocate a unique cursor temp file for {}",
            path.display()
        ),
    ))
}

fn read_cursor(
    path: &Path,
    expected_scope: &CursorScope,
    room_tip: i64,
    allow_unscoped_legacy: bool,
) -> Result<Option<LoadedCursor>, Box<dyn std::error::Error>> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(format!("refusing to read symlink cursor file {}", path.display()).into());
        }
        if !metadata.file_type().is_file() {
            return Err(format!("cursor path {} is not a regular file", path.display()).into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                return Err(format!("refusing hard-linked cursor file {}", path.display()).into());
            }
        }
        harden_cursor_permissions(path)?;
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("failed to read cursor file {}: {error}", path.display()).into())
        }
    };
    if let Ok(seq) = raw.trim().parse::<i64>() {
        if !allow_unscoped_legacy {
            return Err(format!(
                "refusing unscoped legacy integer cursor {}; rerun with --import-legacy-cursor only after verifying its endpoint, room, and agent",
                path.display()
            )
            .into());
        }
        validate_cursor_seq(path, seq, room_tip)?;
        eprintln!(
            "warning: importing unscoped legacy cursor {}; asserting it belongs to this endpoint, room, and agent",
            path.display()
        );
        return Ok(Some(LoadedCursor {
            seq,
            needs_upgrade: true,
            unscoped_legacy: true,
        }));
    }

    let state: CursorState = serde_json::from_str(&raw).map_err(|error| {
        Box::<dyn std::error::Error>::from(format!(
            "invalid cursor file {}: expected a legacy integer or versioned cursor JSON: {error}",
            path.display()
        ))
    })?;
    if state.version != CURSOR_VERSION && state.version != 1 {
        return Err(format!(
            "unsupported cursor version {} in {} (expected {})",
            state.version,
            path.display(),
            CURSOR_VERSION
        )
        .into());
    }
    if !cursor_state_matches_scope(&state, expected_scope) {
        return Err(format!(
            "cursor scope mismatch in {}: expected endpoint={}, room={}, agent={}",
            path.display(),
            expected_scope.endpoint,
            expected_scope.room_id,
            expected_scope.agent_id
        )
        .into());
    }
    validate_cursor_seq(path, state.seq, room_tip)?;
    Ok(Some(LoadedCursor {
        seq: state.seq,
        needs_upgrade: state.version != CURSOR_VERSION,
        unscoped_legacy: false,
    }))
}

fn harden_cursor_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn upgrade_loaded_cursor(
    path: &Path,
    scope: &CursorScope,
    loaded: LoadedCursor,
) -> std::io::Result<()> {
    if !loaded.needs_upgrade {
        return Ok(());
    }
    if loaded.unscoped_legacy {
        import_legacy_cursor_atomic(path, scope, loaded.seq)
    } else {
        write_cursor_atomic(path, scope, loaded.seq)
    }
}

fn advance_cursor(
    cursor: &mut Option<i64>,
    cursor_file: Option<&PathBuf>,
    scope: Option<&CursorScope>,
    seq: i64,
) -> Result<(), ClientError> {
    if cursor.is_some_and(|current| seq < current) {
        return Err(ClientError::Io(cursor_error(format!(
            "refusing to move cursor backwards from {} to {seq}",
            cursor.unwrap_or_default()
        ))));
    }
    if let Some(path) = cursor_file {
        let scope = scope.ok_or_else(|| {
            ClientError::Io(cursor_error("cursor scope is unavailable for checkpoint"))
        })?;
        write_cursor_atomic(path, scope, seq).map_err(ClientError::Io)?;
    }
    *cursor = Some(seq);
    Ok(())
}

fn comparable_path(path: &Path) -> std::io::Result<PathBuf> {
    comparable_path_inner(path, 0)
}

fn comparable_path_inner(path: &Path, symlink_depth: u8) -> std::io::Result<PathBuf> {
    if symlink_depth > 16 {
        return Err(cursor_error("too many symlinks while resolving path"));
    }
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let target = std::fs::read_link(path)?;
        let target = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or_else(|| Path::new(".")).join(target)
        };
        return comparable_path_inner(&target, symlink_depth + 1);
    }
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            match absolute.parent() {
                Some(parent) => match std::fs::canonicalize(parent) {
                    Ok(parent) => Ok(parent.join(
                        absolute
                            .file_name()
                            .ok_or_else(|| cursor_error("path has no file name"))?,
                    )),
                    Err(_) => Ok(absolute),
                },
                None => Ok(absolute),
            }
        }
        Err(error) => Err(error),
    }
}

fn reject_output_cursor_alias(
    output: Option<&PathBuf>,
    cursor_file: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let (Some(output), Some(cursor)) = (output, cursor_file) {
        let comparable_output = comparable_path(output)?;
        let comparable_cursor = comparable_path(cursor)?;
        let mut aliases = comparable_output == comparable_cursor;
        #[cfg(unix)]
        if !aliases {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(output_metadata), Ok(cursor_metadata)) =
                (std::fs::metadata(output), std::fs::metadata(cursor))
            {
                aliases = output_metadata.dev() == cursor_metadata.dev()
                    && output_metadata.ino() == cursor_metadata.ino();
            }
        }
        // Reject a missing-path case-only alias conservatively. This protects
        // default macOS/Windows filesystems and case-folded mounts even when
        // neither path exists yet.
        if !aliases {
            aliases = comparable_output.to_string_lossy().to_lowercase()
                == comparable_cursor.to_string_lossy().to_lowercase();
        }
        if aliases {
            return Err(format!(
                "--output and --cursor-file must be different files (both resolve to {})",
                comparable_output.display()
            )
            .into());
        }
    }
    Ok(())
}

fn open_output_file(
    output: Option<&PathBuf>,
    cursor_file: Option<&PathBuf>,
) -> Result<Option<std::fs::File>, Box<dyn std::error::Error>> {
    let Some(path) = output else {
        return Ok(None);
    };
    reject_output_cursor_alias(output, cursor_file)?;
    // Open without truncating, verify the opened inode is not the cursor, and
    // only then truncate. This closes the hardlink/symlink alias corruption
    // window present in path-only checks.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    if let Some(cursor) = cursor_file {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(cursor_metadata) = std::fs::metadata(cursor) {
                let output_metadata = file.metadata()?;
                if output_metadata.dev() == cursor_metadata.dev()
                    && output_metadata.ino() == cursor_metadata.ino()
                {
                    return Err(format!(
                        "--output and --cursor-file must not refer to the same filesystem object ({})",
                        path.display()
                    )
                    .into());
                }
            }
        }
    }
    file.set_len(0)?;
    Ok(Some(file))
}

fn write_output_line(output: &mut Option<std::fs::File>, rendered: &str) -> std::io::Result<()> {
    match output {
        Some(file) => writeln!(file, "{rendered}"),
        None => {
            let stdout = std::io::stdout();
            writeln!(stdout.lock(), "{rendered}")
        }
    }
}

fn finish_output(output: &mut Option<std::fs::File>) -> std::io::Result<()> {
    match output {
        Some(file) => {
            file.flush()?;
            file.sync_all()
        }
        None => std::io::stdout().lock().flush(),
    }
}

fn append_output_line(
    path: &Path,
    cursor_file: Option<&PathBuf>,
    rendered: &str,
) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    #[cfg(unix)]
    if let Some(cursor) = cursor_file {
        use std::os::unix::fs::MetadataExt;
        if let Ok(cursor_metadata) = std::fs::metadata(cursor) {
            let output_metadata = file.metadata()?;
            if output_metadata.dev() == cursor_metadata.dev()
                && output_metadata.ino() == cursor_metadata.ino()
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "output and cursor refer to the same filesystem object",
                ));
            }
        }
    }
    writeln!(file, "{rendered}")?;
    file.flush()?;
    file.sync_all()
}

fn is_retryable_wait_error(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Io(_)
            | ClientError::ConnectionClosed
            | ClientError::Timeout
            | ClientError::Channel
            | ClientError::Ws(_)
    )
}

fn spawn_wait_presence_watcher(
    client: &CowchatClient,
    room_id: &str,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let mut events = client.subscribe();
    let room_label = room_id.to_string();
    Some(tokio::spawn(async move {
        while let Ok(evt) = events.recv().await {
            let in_room = evt.frame.payload.get("room_id").and_then(|v| v.as_str())
                == Some(room_label.as_str());
            if !in_room {
                continue;
            }
            match evt.frame.frame_type {
                FrameType::AgentJoined => {
                    let name = evt
                        .frame
                        .payload
                        .get("agent")
                        .and_then(|a| a.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    eprintln!("wait: peer {} joined", name);
                }
                FrameType::AgentLeft => {
                    let who = evt
                        .frame
                        .payload
                        .get("agent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    eprintln!("wait: peer {} left", who);
                }
                _ => {}
            }
        }
    }))
}

fn spawn_wait_thinking_watcher(
    client: &CowchatClient,
    room_id: &str,
    self_name: &str,
    secret: Option<Vec<u8>>,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let mut events = client.subscribe();
    let room_label = room_id.to_string();
    let self_name = self_name.to_string();
    Some(tokio::spawn(async move {
        while let Ok(evt) = events.recv().await {
            if evt.frame.frame_type != FrameType::Thinking {
                continue;
            }
            let p = &evt.frame.payload;
            if p.get("room_id").and_then(|v| v.as_str()) != Some(room_label.as_str()) {
                continue;
            }
            let name = p.get("agent_name").and_then(|v| v.as_str()).unwrap_or("?");
            if name == self_name {
                continue;
            }
            let content = p.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let content = decrypt_field(secret.as_deref(), &room_label, content);
            eprintln!("wait: thinking {}: {}", name, content);
        }
    }))
}

#[derive(Clone, Copy)]
struct HistoryFollowOptions<'a> {
    limit: u32,
    since: Option<&'a str>,
    since_seq: Option<i64>,
    kind: Option<&'a str>,
}

async fn history_follow_floor(
    client: &CowchatClient,
    room_id: &str,
    room_tip: i64,
    options: HistoryFollowOptions<'_>,
) -> Result<i64, ClientError> {
    if let Some(seq) = options.since_seq {
        return Ok(seq);
    }
    if options.limit == 0 {
        return Ok(room_tip);
    }
    if let Some(since) = options.since {
        // Resolve the message-id anchor to the first retained successor. The
        // fixed room tip fences concurrent posts; anything newer is recovered
        // by the normal follow loop.
        return Ok(client
            .get_history_since(room_id, 1, None, Some(since))
            .await?
            .into_iter()
            .find(|message| message.seq <= room_tip)
            .map(|message| message.seq.saturating_sub(1))
            .unwrap_or(room_tip));
    }

    // `history --follow` starts with the retained tail, as before, but converts
    // it to a sequence floor so every row through the captured tip is validated
    // and later reconnects can backfill without a race.
    let lookback = room_tip.saturating_sub(i64::from(options.limit));
    Ok(client
        .get_history_filtered(room_id, options.limit, None, None, Some(lookback))
        .await?
        .into_iter()
        .find(|message| message.seq <= room_tip)
        .map(|message| message.seq.saturating_sub(1))
        .unwrap_or(room_tip))
}

#[allow(clippy::too_many_arguments)]
async fn run_wait_follow(
    cli: &Cli,
    room: &str,
    since_seq: Option<&str>,
    heartbeat_secs: u64,
    only_from: Option<&String>,
    not_from: Option<&String>,
    only_kind: Option<&String>,
    show_thinking: bool,
    text: bool,
    output: Option<&PathBuf>,
    cursor_file: Option<&PathBuf>,
    history: Option<HistoryFollowOptions<'_>>,
) -> Result<(), Box<dyn std::error::Error>> {
    reject_output_cursor_alias(output, cursor_file)?;
    let mut cursor = None;
    let mut active_scope: Option<CursorScope> = None;
    let mut cursor_loaded = false;
    let mut output_initialized = false;
    let mut backoff = 1u64;
    let started = std::time::Instant::now();
    let mut last_heartbeat = std::time::Instant::now();

    loop {
        let client = match connect(cli).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!("wait --follow: connect failed: {error}; retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        let room_id = match resolve_room_id(&client, room).await {
            Ok(id) => id,
            Err(error) => {
                eprintln!("wait --follow: room lookup failed: {error}; retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        let room_tip = match client.room_tip(&room_id).await {
            Ok(tip) => tip,
            Err(error) => {
                eprintln!("wait --follow: tip lookup failed: {error}; retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        let resolved_scope = cursor_scope(cli, &room_id)?;
        if active_scope
            .as_ref()
            .is_some_and(|scope| scope != &resolved_scope)
        {
            return Err(
                "wait --follow resolved to a different cursor scope after reconnect".into(),
            );
        }
        active_scope = Some(resolved_scope);

        if !cursor_loaded {
            let loaded = match cursor_file {
                Some(path) => read_cursor(
                    path,
                    active_scope.as_ref().expect("scope was just set"),
                    room_tip,
                    cli.import_legacy_cursor,
                )?,
                None => None,
            };
            cursor = match loaded {
                Some(loaded) => Some(loaded.seq),
                None => {
                    if let Some(history) = history {
                        match history_follow_floor(&client, &room_id, room_tip, history).await {
                            Ok(seq) => Some(seq),
                            Err(error) if is_retryable_wait_error(&error) => {
                                eprintln!(
                                    "history --follow: initial catch-up failed: {error}; retrying in {backoff}s"
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                                backoff = (backoff * 2).min(30);
                                continue;
                            }
                            Err(error) => return Err(Box::new(error)),
                        }
                    } else {
                        match since_seq {
                            None => Some(room_tip),
                            Some(s)
                                if s.eq_ignore_ascii_case("tip")
                                    || s.eq_ignore_ascii_case("auto") =>
                            {
                                Some(room_tip)
                            }
                            Some(s) => Some(s.parse::<i64>().map_err(|error| {
                                format!("--since-seq must be an integer, 'tip', or 'auto': {error}")
                            })?),
                        }
                    }
                }
            };
            if let Some(seq) = cursor {
                validate_sequence_floor("--since-seq", seq, room_tip)?;
                if let Some(path) = cursor_file {
                    if loaded.is_none_or(|loaded| loaded.needs_upgrade) {
                        let scope = active_scope.as_ref().expect("scope was just set");
                        let result = match loaded {
                            Some(loaded) => upgrade_loaded_cursor(path, scope, loaded),
                            None => write_cursor_atomic(path, scope, seq),
                        };
                        result.map_err(|error| {
                            Box::<dyn std::error::Error>::from(format!(
                                "failed to initialize follow cursor file {}: {error}",
                                path.display()
                            ))
                        })?;
                    }
                }
            }
            cursor_loaded = true;
        } else if let Some(seq) = cursor {
            validate_sequence_floor(
                cursor_file
                    .map(|path| format!("cursor file {}", path.display()))
                    .as_deref()
                    .unwrap_or("follow sequence"),
                seq,
                room_tip,
            )?;
        }
        if let Err(error) = client.join_room(&room_id).await {
            eprintln!("wait --follow: join failed: {error}; retrying in {backoff}s");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
            continue;
        }
        let _ = client.set_presence("waiting", None, None).await;
        if !output_initialized {
            if history.is_some() {
                let mut output_file = open_output_file(output, None)?;
                finish_output(&mut output_file)?;
                if output.is_none() {
                    println!("--- streaming room history (Ctrl+C to stop) ---");
                }
            }
            output_initialized = true;
        }
        backoff = 1;

        let connection_result: Result<(), ClientError> = async {
            loop {
                // Pulling history before waiting makes the cursor authoritative
                // across disconnects and broadcast lag. Advance over filtered,
                // self, thinking, and system rows too, so none can pin recovery.
                let history_floor = cursor.unwrap_or(0);
                let captured_tip = client.room_tip(&room_id).await?;
                let batch = client
                    .get_contiguous_history_page(&room_id, history_floor, captured_tip, 500)
                    .await?;
                if batch.is_empty() {
                    // The push is only a latency signal. Always loop back through
                    // fixed-tip contiguous history before emitting/checkpointing;
                    // that preserves thinking/system rows that arrived in the
                    // same burst as the chat message which woke us.
                    let _ = client.wait_for_message(&room_id, 5, cursor).await?;
                    continue;
                }

                for message in batch {
                    if cursor.is_some_and(|seq| message.seq <= seq) {
                        continue;
                    }
                    let row_type = message.metadata.get("type").and_then(|v| v.as_str());
                    if history.is_none() && row_type == Some("thinking") {
                        if show_thinking && !client.is_self_message(&message) {
                            eprintln!("wait: thinking {}: {}", message.agent_name, message.content);
                        }
                        advance_cursor(
                            &mut cursor,
                            cursor_file,
                            active_scope.as_ref(),
                            message.seq,
                        )?;
                        continue;
                    }
                    if history.is_none()
                        && (row_type == Some("system") || client.is_self_message(&message))
                    {
                        advance_cursor(
                            &mut cursor,
                            cursor_file,
                            active_scope.as_ref(),
                            message.seq,
                        )?;
                        continue;
                    }
                    let wanted_kind = history
                        .and_then(|options| options.kind)
                        .or_else(|| only_kind.map(String::as_str));
                    if only_from.is_some_and(|name| message.agent_name != *name)
                        || not_from.is_some_and(|name| message.agent_name == *name)
                        || wanted_kind.is_some_and(|kind| {
                            message.metadata.get("kind").and_then(|v| v.as_str()) != Some(kind)
                        })
                    {
                        advance_cursor(
                            &mut cursor,
                            cursor_file,
                            active_scope.as_ref(),
                            message.seq,
                        )?;
                        continue;
                    }

                    let rendered = if text || history.is_some() {
                        format_message(&message)
                    } else {
                        serde_json::to_string(&message).unwrap_or_default()
                    };
                    if let Some(path) = output {
                        append_output_line(path, cursor_file, &rendered)
                            .map_err(ClientError::Io)?;
                    } else {
                        println!("{rendered}");
                        std::io::stdout().flush().map_err(ClientError::Io)?;
                    }
                    advance_cursor(&mut cursor, cursor_file, active_scope.as_ref(), message.seq)?;
                    if history.is_none()
                        && message.metadata.get("kind").and_then(|v| v.as_str())
                            == Some(KIND_CONVERSATION_END)
                    {
                        eprintln!("Peer ended the conversation.");
                        std::process::exit(3);
                    }
                }

                if heartbeat_secs > 0
                    && last_heartbeat.elapsed() >= std::time::Duration::from_secs(heartbeat_secs)
                {
                    eprintln!(
                        "wait: alive {}s room={} since_seq={} mode=follow",
                        started.elapsed().as_secs(),
                        room_id,
                        cursor.unwrap_or(0)
                    );
                    last_heartbeat = std::time::Instant::now();
                }
            }
        }
        .await;
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.set_presence("idle", None, None),
        )
        .await;
        if let Err(error) = connection_result {
            // Cursor/output I/O and an unrecoverable broadcast gap cannot be
            // repaired by reconnecting. Retrying would keep streaming from a
            // non-contiguous floor and could silently replay or skip data, so
            // fail closed and let the operator fix the path.
            if matches!(
                error,
                ClientError::Io(_)
                    | ClientError::EventStreamLagged { .. }
                    | ClientError::HistoryGap { .. }
                    | ClientError::HistoryCursorAhead { .. }
            ) {
                return Err(Box::new(error));
            }
            eprintln!("wait --follow: connection lost: {error}; retrying in {backoff}s");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    }
}

fn print_shell_help() {
    println!("Interactive shell commands:");
    println!("  /help                 Show this help");
    println!("  /join <room>          Join room (id or exact name) and make it active");
    println!("  /leave [room]         Leave active room (or explicit room)");
    println!("  /room                 Show current active room");
    println!("  /rooms                List rooms");
    println!("  /agents               List agents in active room");
    println!("  /history [limit]      Show room history (default 20)");
    println!("  /send <message>       Send message to active room");
    println!("  /quit                 Exit shell");
    println!("  <text>                Shortcut for /send <text>");
}

fn print_shell_prompt(current_room: Option<&str>) {
    let room = current_room.unwrap_or("no-room");
    print!("cowchat[{room}]> ");
    let _ = std::io::stdout().flush();
}

async fn run_shell(
    cli: &Cli,
    start_room: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = connect(cli).await?;
    let room_secret = resolve_room_secret(cli);
    let mut current_room: Option<String> = None;

    if let Some(room_ref) = start_room {
        let room_id = resolve_room_id(&client, room_ref).await?;
        client.join_room(&room_id).await?;
        println!("Joined room: {}", room_id);
        current_room = Some(room_id);
    }

    println!(
        "Connected as '{}' (agent_id: {})",
        client.agent_name, client.agent_id
    );
    print_shell_help();

    let mut stdin_lines = BufReader::new(tokio::io::stdin()).lines();
    let mut events = client.subscribe();

    loop {
        print_shell_prompt(current_room.as_deref());

        tokio::select! {
            line = stdin_lines.next_line() => {
                let Some(line) = line? else {
                    println!();
                    break;
                };

                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                if let Some(command_text) = input.strip_prefix('/') {
                    let (cmd, rest) = match command_text.split_once(' ') {
                        Some((cmd, rest)) => (cmd.trim(), rest.trim()),
                        None => (command_text.trim(), ""),
                    };

                    match cmd {
                        "help" => print_shell_help(),
                        "join" => {
                            if rest.is_empty() {
                                println!("Usage: /join <room-id-or-name>");
                                continue;
                            }
                            match resolve_room_id(&client, rest).await {
                                Ok(room_id) => {
                                    match client.join_room(&room_id).await {
                                        Ok(_) => {
                                            println!("Joined room: {}", room_id);
                                            current_room = Some(room_id);
                                        }
                                        Err(e) => println!("Join failed: {}", e),
                                    }
                                }
                                Err(e) => println!("Join failed: {}", e),
                            }
                        }
                        "leave" => {
                            let target_room = if rest.is_empty() {
                                current_room.clone()
                            } else {
                                match resolve_room_id(&client, rest).await {
                                    Ok(id) => Some(id),
                                    Err(e) => {
                                        println!("Leave failed: {}", e);
                                        None
                                    }
                                }
                            };

                            if let Some(room_id) = target_room {
                                match client.leave_room(&room_id).await {
                                    Ok(_) => {
                                        println!("Left room: {}", room_id);
                                        if current_room.as_deref() == Some(room_id.as_str()) {
                                            current_room = None;
                                        }
                                    }
                                    Err(e) => println!("Leave failed: {}", e),
                                }
                            } else {
                                println!("No active room to leave.");
                            }
                        }
                        "room" => {
                            match current_room.as_deref() {
                                Some(room_id) => println!("Active room: {}", room_id),
                                None => println!("No active room. Use /join <room> first."),
                            }
                        }
                        "rooms" => {
                            match client.list_rooms(None).await {
                                Ok(rooms) => {
                                    if rooms.is_empty() {
                                        println!("No rooms found.");
                                    } else {
                                        println!("{:<38} {:<20} {:<10} DESCRIPTION", "ID", "NAME", "TYPE");
                                        println!("{}", "-".repeat(80));
                                        for room in rooms {
                                            let room_type = if room.ephemeral { "ephemeral" } else { "permanent" };
                                            let desc = room.description.as_deref().unwrap_or("");
                                            println!("{:<38} {:<20} {:<10} {}", room.room_id, room.name, room_type, desc);
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to list rooms: {}", e),
                            }
                        }
                        "agents" => {
                            match client.list_agents(current_room.as_deref()).await {
                                Ok(agents) => {
                                    if agents.is_empty() {
                                        println!("No agents connected.");
                                    } else {
                                        println!(
                                            "{:<38} {:<16} {:<10} {:<10} DETAIL",
                                            "AGENT ID", "NAME", "STATUS", "PROGRESS"
                                        );
                                        println!("{}", "-".repeat(100));
                                        for agent in agents {
                                            let status = agent.status.as_deref().unwrap_or("-");
                                            let progress = agent
                                                .progress
                                                .map(|p| format!("{}%", p))
                                                .unwrap_or_default();
                                            let detail = agent.status_detail.as_deref().unwrap_or("");
                                            println!(
                                                "{:<38} {:<16} {:<10} {:<10} {}",
                                                agent.agent_id, agent.name, status, progress, detail
                                            );
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to list agents: {}", e),
                            }
                        }
                        "history" => {
                            let limit = if rest.is_empty() {
                                20
                            } else {
                                match rest.parse::<u32>() {
                                    Ok(v) => v,
                                    Err(_) => {
                                        println!("Usage: /history [limit]");
                                        continue;
                                    }
                                }
                            };

                            let Some(room_id) = current_room.as_deref() else {
                                println!("No active room. Use /join <room> first.");
                                continue;
                            };

                            match client.get_history(room_id, limit, None).await {
                                Ok(messages) => {
                                    for msg in messages {
                                        println!("{}", format_message(&msg));
                                    }
                                }
                                Err(e) => println!("Failed to load history: {}", e),
                            }
                        }
                        "send" => {
                            if rest.is_empty() {
                                println!("Usage: /send <message>");
                                continue;
                            }
                            if let Some(room_id) = current_room.as_deref() {
                                if let Err(e) = client.send_message(room_id, rest, None, vec![]).await {
                                    println!("Send failed: {}", e);
                                }
                            } else {
                                println!("No active room. Use /join <room> first.");
                            }
                        }
                        "quit" | "exit" => break,
                        _ => {
                            println!("Unknown command: /{} (try /help)", cmd);
                        }
                    }
                } else if let Some(room_id) = current_room.as_deref() {
                    if let Err(e) = client.send_message(room_id, input, None, vec![]).await {
                        println!("Send failed: {}", e);
                    }
                } else {
                    println!("No active room. Use /join <room> first.");
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Some(active_room) = current_room.as_deref() {
                            if let Some(event_room) = event.frame.payload.get("room_id").and_then(|v| v.as_str()) {
                                if event_room != active_room {
                                    continue;
                                }
                            }
                        }
                        println!();
                        print_event(&event.frame, room_secret.as_deref());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        println!("\n[warn] event stream lagged (dropped {} events)", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        println!("\n[event stream closed]");
                        break;
                    }
                }
            }
        }
    }

    println!("Goodbye.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cli = Cli::parse();
    let is_named_agent_invocation = cli.name != "cli";
    if (command_requires_stable_agent_id(&cli.command)
        || (is_named_agent_invocation && command_opens_connection(&cli.command)))
        && resolve_agent_id(&cli).is_none()
    {
        return Err(
            "this agent-facing command requires a stable identity; pass --agent-id <UNIQUE_TASK_AGENT_ID> or set COWCHAT_AGENT_ID"
                .into(),
        );
    }

    match &cli.command {
        Commands::Send {
            room,
            message,
            reply_to,
            kind,
            cursor_file,
            end,
        } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;
            client.join_room(&room_id).await?;
            if let Some(path) = cursor_file {
                let scope = cursor_scope(&cli, &room_id)?;
                let tip = client.room_tip(&room_id).await?;
                let loaded = read_cursor(path, &scope, tip, cli.import_legacy_cursor)?;
                if let Some(loaded) = loaded {
                    if loaded.needs_upgrade {
                        upgrade_loaded_cursor(path, &scope, loaded).map_err(|error| {
                            Box::<dyn std::error::Error>::from(format!(
                                "failed to upgrade cursor file {} before send: {error}",
                                path.display()
                            ))
                        })?;
                    }
                } else {
                    // Never infer that everything at the current tip was read:
                    // a peer message can land after catch-up but before this send.
                    // A precise history catch-up creates the cursor first; zero is
                    // the safe at-least-once fallback when callers skip that step.
                    write_cursor_atomic(path, &scope, 0).map_err(|error| {
                        Box::<dyn std::error::Error>::from(format!(
                            "failed to initialize cursor file {} before send: {error}",
                            path.display()
                        ))
                    })?;
                }
            }
            // `--end` is sugar for `--kind conversation_end` (the two conflict at
            // the arg level, so at most one is set).
            let kind = if *end {
                Some(KIND_CONVERSATION_END.to_string())
            } else {
                kind.clone()
            };
            let msg = match &kind {
                Some(k) => {
                    client
                        .send_message_with_metadata(
                            &room_id,
                            message,
                            reply_to.as_deref(),
                            vec![],
                            serde_json::json!({ "kind": k }),
                        )
                        .await?
                }
                None => {
                    client
                        .send_message(&room_id, message, reply_to.as_deref(), vec![])
                        .await?
                }
            };
            println!("{}", format_message(&msg));
        }

        Commands::Thinking { room, content } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;
            client.join_room(&room_id).await?;
            let msg = client.thinking(&room_id, content).await?;
            println!("{}", format_message(&msg));
        }

        Commands::Rooms { action } => {
            let client = connect(&cli).await?;
            match action {
                RoomAction::List { parent } => {
                    let rooms = client.list_rooms(parent.as_deref()).await?;
                    if rooms.is_empty() {
                        println!("No rooms found.");
                    } else {
                        println!(
                            "{:<38} {:<20} {:<10} {:<8} {:<20} DESCRIPTION",
                            "ID", "NAME", "TYPE", "MEMBERS", "LAST ACTIVITY"
                        );
                        println!("{}", "-".repeat(130));
                        for room in rooms {
                            let room_type = if room.ephemeral {
                                "ephemeral"
                            } else {
                                "permanent"
                            };
                            let desc = room.description.as_deref().unwrap_or("");
                            let members = room
                                .member_count
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "-".to_string());
                            let activity = room
                                .last_activity
                                .map(|t| t.format("%H:%M:%S").to_string())
                                .unwrap_or_else(|| "-".to_string());
                            println!(
                                "{:<38} {:<20} {:<10} {:<8} {:<20} {}",
                                room.room_id, room.name, room_type, members, activity, desc
                            );
                        }
                    }
                }
                RoomAction::Create {
                    room_name,
                    description,
                    parent,
                    ephemeral,
                    public,
                    encrypted,
                } => {
                    let room = client
                        .create_room_with_options(
                            room_name,
                            description.as_deref(),
                            parent.as_deref(),
                            *ephemeral,
                            *public,
                            *encrypted,
                        )
                        .await?;
                    println!("Created room: {} ({})", room.name, room.room_id);
                    if room.ephemeral {
                        println!("  Type: ephemeral (auto-deleted when empty)");
                    }
                    println!(
                        "  Visibility: {}",
                        if room.visibility == "public" {
                            "public (any key can find & join)"
                        } else {
                            "private (owning key or keyless-local boundary)"
                        }
                    );
                    if room.encrypted {
                        println!("  End-to-end encrypted: members need a shared room key");
                        if resolve_room_secret(&cli).is_none() {
                            println!(
                                "  Note: no room key set — pass --room-key or set $COWCHAT_ROOM_KEY to send/read here"
                            );
                        }
                    }
                }
                RoomAction::Info { room_id } => {
                    let resolved = resolve_room_id(&client, room_id).await?;
                    let info = client.room_info(&resolved).await?;
                    println!("{}", serde_json::to_string_pretty(&info)?);
                }
                RoomAction::Tip { room } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let seq = client.room_tip(&room_id).await?;
                    println!("{}", seq);
                }
                RoomAction::Rename { room, new_name } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let updated = client.rename_room(&room_id, new_name).await?;
                    println!("Renamed room: {} ({})", updated.name, updated.room_id);
                }
                RoomAction::Destroy { room, yes } => {
                    if !yes {
                        return Err(
                            "room destruction is irreversible; re-run with --yes to confirm".into(),
                        );
                    }
                    let room_id = resolve_room_id(&client, room).await?;
                    client.destroy_room(&room_id).await?;
                    println!("Destroyed room: {}", room_id);
                }
            }
        }

        Commands::Agents { room } => {
            let client = connect(&cli).await?;
            let agents = client.list_agents(room.as_deref()).await?;

            // If --room is set, also pull recent history so we can surface
            // agents who've been posting recently even if they're not currently
            // connected (each CLI invocation registers + disconnects, so an
            // active reviewer flickers in and out of the live agents list).
            let (last_in_room, room_id_for_history): (
                std::collections::HashMap<String, (i64, String)>,
                Option<String>,
            ) = if let Some(r) = room {
                let room_id = resolve_room_id(&client, r).await?;
                let hist = client
                    .get_history(&room_id, 200, None)
                    .await
                    .unwrap_or_default();
                let mut map = std::collections::HashMap::new();
                for m in hist.iter().rev() {
                    // Iterate newest-first; keep first sighting per agent_name.
                    map.entry(m.agent_name.clone())
                        .or_insert((m.seq, m.timestamp.format("%H:%M:%S").to_string()));
                }
                (map, Some(room_id))
            } else {
                (std::collections::HashMap::new(), None)
            };

            let show_room_activity = room_id_for_history.is_some();
            let live_names: std::collections::HashSet<String> =
                agents.iter().map(|a| a.name.clone()).collect();

            if agents.is_empty() && last_in_room.is_empty() {
                println!("No agents connected; no recent activity in room.");
            } else {
                if show_room_activity {
                    println!(
                        "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} DETAIL",
                        "STATE", "AGENT ID", "NAME", "STATUS", "PROG", "ACTIVE", "LAST_SEQ"
                    );
                    println!("{}", "-".repeat(130));
                } else {
                    println!(
                        "{:<38} {:<16} {:<10} {:<8} {:<10} DETAIL",
                        "AGENT ID", "NAME", "STATUS", "PROG", "ACTIVE"
                    );
                    println!("{}", "-".repeat(110));
                }

                // First: currently connected agents.
                for agent in &agents {
                    let status = agent.status.as_deref().unwrap_or("-");
                    let progress = agent
                        .progress
                        .map(|p| format!("{}%", p))
                        .unwrap_or_default();
                    let active = agent
                        .last_active
                        .map(|t| t.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let detail = agent.status_detail.as_deref().unwrap_or("");
                    if show_room_activity {
                        let (last_seq_s, _last_ts_s) = last_in_room
                            .get(&agent.name)
                            .cloned()
                            .unwrap_or((0, "-".to_string()));
                        let last_seq = if last_seq_s > 0 {
                            format!("#{}", last_seq_s)
                        } else {
                            "-".to_string()
                        };
                        println!(
                            "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} {}",
                            "LIVE",
                            agent.agent_id,
                            agent.name,
                            status,
                            progress,
                            active,
                            last_seq,
                            detail
                        );
                    } else {
                        println!(
                            "{:<38} {:<16} {:<10} {:<8} {:<10} {}",
                            agent.agent_id, agent.name, status, progress, active, detail
                        );
                    }
                }

                // Then: agents seen in recent history but NOT currently connected.
                // These are the ghosts the user actually cares about: "has codex
                // been here recently even though they just disconnected?"
                if show_room_activity {
                    for (name, (seq, ts)) in &last_in_room {
                        if live_names.contains(name) {
                            continue;
                        }
                        println!(
                            "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} (last seen via history)",
                            "RECENT",
                            "-",
                            name,
                            "-",
                            "",
                            ts,
                            format!("#{}", seq)
                        );
                    }
                }
            }
        }

        Commands::History {
            room,
            limit,
            follow,
            since,
            since_seq,
            kind,
            output,
            cursor_file,
        } => {
            reject_output_cursor_alias(output.as_ref(), cursor_file.as_ref())?;
            if *follow {
                run_wait_follow(
                    &cli,
                    room,
                    None,
                    30,
                    None,
                    None,
                    None,
                    false,
                    true,
                    output.as_ref(),
                    None,
                    Some(HistoryFollowOptions {
                        limit: *limit,
                        since: since.as_deref(),
                        since_seq: *since_seq,
                        kind: kind.as_deref(),
                    }),
                )
                .await?;
                return Ok(());
            }
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;

            if cursor_file.is_some() && since.is_some() {
                return Err(
                    "history --cursor-file cannot be combined with --since; use --since-seq to seed a missing cursor"
                        .into(),
                );
            }
            let mut written = 0usize;
            if let Some(path) = cursor_file {
                let captured_tip = client.room_tip(&room_id).await?;
                let scope = cursor_scope(&cli, &room_id)?;
                let loaded = read_cursor(path, &scope, captured_tip, cli.import_legacy_cursor)?;
                let start_seq = loaded.map(|cursor| cursor.seq).or(*since_seq).unwrap_or(0);
                validate_cursor_seq(path, start_seq, captured_tip)?;
                let mut output_file = open_output_file(output.as_ref(), cursor_file.as_ref())?;
                let mut processed_seq = start_seq;
                while processed_seq < captured_tip {
                    let page = client
                        .get_contiguous_history_page(&room_id, processed_seq, captured_tip, *limit)
                        .await?;
                    for message in &page {
                        let matches = match kind {
                            Some(kind) => {
                                message
                                    .metadata
                                    .get("kind")
                                    .and_then(|value| value.as_str())
                                    == Some(kind.as_str())
                            }
                            None => true,
                        };
                        if matches {
                            write_output_line(&mut output_file, &format_message(message))?;
                            written += 1;
                        }
                    }
                    processed_seq = page
                        .last()
                        .map(|message| message.seq)
                        .ok_or("contiguous history page unexpectedly empty")?;
                }
                finish_output(&mut output_file)?;
                let result = match loaded {
                    Some(loaded) if loaded.unscoped_legacy => {
                        import_legacy_cursor_atomic(path, &scope, captured_tip)
                    }
                    _ => write_cursor_atomic(path, &scope, captured_tip),
                };
                result.map_err(|error| {
                    Box::<dyn std::error::Error>::from(format!(
                        "failed to checkpoint history cursor file {}: {error}",
                        path.display()
                    ))
                })?;
            } else {
                let messages = if let Some(start_seq) = *since_seq {
                    let captured_tip = client.room_tip(&room_id).await?;
                    client
                        .get_contiguous_history_page(&room_id, start_seq, captured_tip, *limit)
                        .await?
                } else {
                    client
                        .get_history_filtered(&room_id, *limit, None, since.as_deref(), None)
                        .await?
                };
                let mut output_file = open_output_file(output.as_ref(), None)?;
                for message in &messages {
                    let matches = match kind {
                        Some(kind) => {
                            message
                                .metadata
                                .get("kind")
                                .and_then(|value| value.as_str())
                                == Some(kind.as_str())
                        }
                        None => true,
                    };
                    if matches {
                        write_output_line(&mut output_file, &format_message(message))?;
                        written += 1;
                    }
                }
                finish_output(&mut output_file)?;
            }
            if let Some(path) = output {
                eprintln!("wrote {written} entries to {}", path.display());
            }
        }

        Commands::Wait {
            room,
            timeout,
            json: _json,
            text,
            since_seq,
            loop_,
            follow,
            idle_timeout,
            heartbeat_secs,
            only_from,
            not_from,
            only_kind,
            show_thinking,
            output,
            drain,
            cursor_file,
        } => {
            reject_output_cursor_alias(output.as_ref(), cursor_file.as_ref())?;
            if *follow {
                run_wait_follow(
                    &cli,
                    room,
                    since_seq.as_deref(),
                    *heartbeat_secs,
                    only_from.as_ref(),
                    not_from.as_ref(),
                    only_kind.as_ref(),
                    *show_thinking,
                    *text,
                    output.as_ref(),
                    cursor_file.as_ref(),
                    None,
                )
                .await?;
                return Ok(());
            }
            let mut client = connect(&cli).await?;
            let mut room_id = resolve_room_id(&client, room).await?;
            client.join_room(&room_id).await?;
            let initial_tip = client.room_tip(&room_id).await?;
            let scope = cursor_scope(&cli, &room_id)?;

            // A cursor file, when present, is the source of truth for the read
            // floor: it holds the highest seq we've actually processed, so the
            // floor never jumps ahead to our own sent message. It overrides
            // --since-seq, which then only seeds the first run (file absent).
            let loaded_cursor = match cursor_file.as_ref() {
                Some(path) => read_cursor(path, &scope, initial_tip, cli.import_legacy_cursor)?,
                None => None,
            };
            let cursor_seq = loaded_cursor.map(|cursor| cursor.seq);

            // Resolve `--since-seq tip|auto` to the room's current tip. Done
            // BEFORE the wait subscribes so we don't miss anything arriving
            // between the tip read and the subscribe (the SDK's wait subscribes
            // first, then checks history with since_seq — that closes the race).
            let resolved_since_seq: Option<i64> = if let Some(seq) = cursor_seq {
                Some(seq)
            } else {
                match since_seq.as_deref() {
                    // A requested cursor file must have a durable initial floor
                    // even when --since-seq is omitted. Otherwise an idle timeout
                    // leaves the file absent and the next invocation can jump past
                    // a message that landed in the re-arm gap.
                    None if cursor_file.is_some() => Some(initial_tip),
                    // A bare persistent loop still needs an in-memory durable
                    // history floor so transport reconnect cannot miss a peer
                    // message posted while the socket was down.
                    None if *loop_ => Some(initial_tip),
                    None => None,
                    Some(s) if s.eq_ignore_ascii_case("tip") || s.eq_ignore_ascii_case("auto") => {
                        Some(initial_tip)
                    }
                    Some(s) => Some(s.parse::<i64>().map_err(|e| {
                        Box::<dyn std::error::Error>::from(format!(
                            "--since-seq must be an integer, 'tip', or 'auto': {}",
                            e
                        ))
                    })?),
                }
            };
            if let Some(seq) = resolved_since_seq {
                let label = cursor_file
                    .as_ref()
                    .map(|path| format!("cursor file {}", path.display()))
                    .unwrap_or_else(|| "--since-seq".to_string());
                validate_sequence_floor(&label, seq, initial_tip)?;
            }

            // Persist the initial floor before blocking. From this point onward,
            // the exact same command is safe to re-run after a timeout: a peer
            // message that lands between invocations remains strictly above the
            // cursor instead of being swallowed by a fresh `tip` lookup.
            if cursor_seq.is_none() {
                if let (Some(path), Some(seq)) = (cursor_file.as_ref(), resolved_since_seq) {
                    write_cursor_atomic(path, &scope, seq).map_err(|error| {
                        Box::<dyn std::error::Error>::from(format!(
                            "failed to initialize cursor file {}: {error}",
                            path.display()
                        ))
                    })?;
                }
            } else if loaded_cursor.is_some_and(|cursor| cursor.needs_upgrade) {
                if let (Some(path), Some(loaded)) = (cursor_file.as_ref(), loaded_cursor) {
                    upgrade_loaded_cursor(path, &scope, loaded).map_err(|error| {
                        Box::<dyn std::error::Error>::from(format!(
                            "failed to upgrade cursor file {}: {error}",
                            path.display()
                        ))
                    })?;
                }
            }

            // Announce we're waiting so other agents in the room know someone is blocked.
            // Best-effort — don't fail the wait if presence broadcast fails.
            let _ = client.set_presence("waiting", None, None).await;

            let effective_timeout = if *timeout == 0 { 86400 } else { *timeout }; // 0 = 24h

            // Heartbeat task: periodically prints to stderr so tool wrappers that kill
            // silent processes see liveness. Aborted as soon as wait returns.
            let heartbeat_task = if *heartbeat_secs > 0 {
                let interval = *heartbeat_secs;
                let room_label = room_id.clone();
                let since_label = resolved_since_seq
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let mode_label = if *loop_ { "loop" } else { "once" };
                Some(tokio::spawn(async move {
                    let started = std::time::Instant::now();
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval));
                    tick.tick().await; // skip the immediate first tick
                    loop {
                        tick.tick().await;
                        eprintln!(
                            "wait: alive {}s room={} since_seq={} mode={}",
                            started.elapsed().as_secs(),
                            room_label,
                            since_label,
                            mode_label,
                        );
                    }
                }))
            } else {
                None
            };

            // Presence-watcher task: prints peer join/leave to stderr while the
            // wait blocks, so a waiting agent (or the human) can see the other
            // side arrive and know to keep waiting instead of concluding "gone".
            // Uses its own event subscription, independent of the SDK wait loop.
            // Gated by the same quiet switch as heartbeats.
            let mut presence_task =
                spawn_wait_presence_watcher(&client, &room_id, *heartbeat_secs > 0);

            // Thinking-watcher task: with --show-thinking, print peers' thinking
            // pulses to stderr for live visibility, WITHOUT waking the wait (it
            // still only returns on a real chat message). Own pulses are skipped;
            // content is decrypted if a room key is configured.
            let room_secret = resolve_room_secret(&cli);
            let mut thinking_task = spawn_wait_thinking_watcher(
                &client,
                &room_id,
                &cli.name,
                room_secret.clone(),
                *show_thinking,
            );

            // Helper closure: does a candidate message match all filters?
            let matches = |msg: &ChatMessage| -> bool {
                if let Some(want) = only_from {
                    if &msg.agent_name != want {
                        return false;
                    }
                }
                if let Some(skip) = not_from {
                    if &msg.agent_name == skip {
                        return false;
                    }
                }
                if let Some(want_kind) = only_kind {
                    let got_kind = msg.metadata.get("kind").and_then(|v| v.as_str());
                    if got_kind != Some(want_kind.as_str()) {
                        return false;
                    }
                }
                true
            };

            // Inner loop: with --loop, keep advancing the bookmark until the
            // returned message passes all filters. Without --loop, do at most
            // one underlying wait call. In --loop mode the loop never returns on
            // its own until a match arrives, so an optional idle deadline races it.
            // `latest_seq` mirrors the bookmark out of the future (which the
            // select! may drop) so the idle-expiry path can print an accurate
            // resume point even after filters advanced past some messages.
            // i64::MIN is the "never advanced" sentinel (resume from `tip`).
            let latest_seq =
                std::sync::atomic::AtomicI64::new(resolved_since_seq.unwrap_or(i64::MIN));
            let wait_loop = async {
                let mut cursor = resolved_since_seq;
                let mut backoff = 1u64;
                loop {
                    let result = client
                        .wait_for_message(&room_id, effective_timeout, cursor)
                        .await;
                    match result {
                        Ok(Some(msg)) => {
                            backoff = 1;
                            if matches(&msg) {
                                break Ok(Some(msg));
                            }
                            // A filter rejection is still a processed row. Persist it
                            // before the next await so an idle timeout cannot cancel
                            // this future while leaving the durable cursor behind.
                            advance_cursor(
                                &mut cursor,
                                cursor_file.as_ref(),
                                Some(&scope),
                                msg.seq,
                            )?;
                            latest_seq.store(msg.seq, std::sync::atomic::Ordering::Relaxed);
                            // Continuing only makes sense in --loop mode; a one-shot
                            // filtered wait reports no match after checkpointing.
                            if !*loop_ {
                                break Ok(None);
                            }
                        }
                        Ok(None) if *loop_ => continue,
                        Ok(None) => break Ok(None),
                        Err(error) if *loop_ && is_retryable_wait_error(&error) => {
                            eprintln!(
                                "wait --loop: connection lost: {error}; retrying in {backoff}s"
                            );
                            if let Some(task) = presence_task.take() {
                                task.abort();
                            }
                            if let Some(task) = thinking_task.take() {
                                task.abort();
                            }

                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                                let replacement = match connect(&cli).await {
                                    Ok(replacement) => replacement,
                                    Err(reconnect_error) => {
                                        eprintln!(
                                            "wait --loop: reconnect failed: {reconnect_error}; retrying in {}s",
                                            (backoff * 2).min(30)
                                        );
                                        backoff = (backoff * 2).min(30);
                                        continue;
                                    }
                                };
                                let replacement_room_id = match resolve_room_id(&replacement, room)
                                    .await
                                {
                                    Ok(id) => id,
                                    Err(reconnect_error) => {
                                        eprintln!(
                                            "wait --loop: room lookup failed: {reconnect_error}; retrying in {}s",
                                            (backoff * 2).min(30)
                                        );
                                        backoff = (backoff * 2).min(30);
                                        continue;
                                    }
                                };
                                if replacement_room_id != scope.room_id {
                                    return Err(ClientError::Io(cursor_error(
                                        "wait --loop resolved to a different cursor room after reconnect",
                                    )));
                                }
                                if let Err(reconnect_error) =
                                    replacement.join_room(&replacement_room_id).await
                                {
                                    eprintln!(
                                        "wait --loop: join failed: {reconnect_error}; retrying in {}s",
                                        (backoff * 2).min(30)
                                    );
                                    backoff = (backoff * 2).min(30);
                                    continue;
                                }
                                let replacement_tip = match replacement
                                    .room_tip(&replacement_room_id)
                                    .await
                                {
                                    Ok(tip) => tip,
                                    Err(reconnect_error) => {
                                        eprintln!(
                                            "wait --loop: tip lookup failed: {reconnect_error}; retrying in {}s",
                                            (backoff * 2).min(30)
                                        );
                                        backoff = (backoff * 2).min(30);
                                        continue;
                                    }
                                };
                                if let Some(seq) = cursor {
                                    let label = cursor_file
                                        .as_ref()
                                        .map(|path| format!("cursor file {}", path.display()))
                                        .unwrap_or_else(|| "wait sequence".to_string());
                                    validate_sequence_floor(&label, seq, replacement_tip)
                                        .map_err(ClientError::Io)?;
                                }

                                client = replacement;
                                room_id = replacement_room_id;
                                let _ = client.set_presence("waiting", None, None).await;
                                presence_task = spawn_wait_presence_watcher(
                                    &client,
                                    &room_id,
                                    *heartbeat_secs > 0,
                                );
                                thinking_task = spawn_wait_thinking_watcher(
                                    &client,
                                    &room_id,
                                    &cli.name,
                                    room_secret.clone(),
                                    *show_thinking,
                                );
                                backoff = 1;
                                break;
                            }
                        }
                        Err(error) => break Err(error),
                    }
                }
            };

            // `idle_expired` is only reachable in --loop mode with a non-zero
            // deadline; otherwise we just await the loop.
            let mut idle_expired = false;
            let matched: Result<Option<ChatMessage>, _> = if *loop_ && *idle_timeout > 0 {
                // On expiry the loser future is dropped; a message that landed in
                // the broadcast buffer in that instant is discarded, but the resume
                // hint below (from --since-seq) lets the caller catch up on re-run.
                tokio::select! {
                    r = wait_loop => r,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(*idle_timeout)) => {
                        idle_expired = true;
                        Ok(None)
                    }
                }
            } else {
                wait_loop.await
            };

            if let Some(task) = heartbeat_task {
                task.abort();
            }
            if let Some(task) = presence_task.take() {
                task.abort();
            }
            if let Some(task) = thinking_task.take() {
                task.abort();
            }

            // Reset presence regardless of outcome so other agents see us return to idle.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                client.set_presence("idle", None, None),
            )
            .await;

            if idle_expired {
                let resume = match latest_seq.load(std::sync::atomic::Ordering::Relaxed) {
                    i64::MIN => "tip".to_string(),
                    n => n.to_string(),
                };
                if cursor_file.is_some() {
                    eprintln!(
                        "No message for {}s — the turn may be stalled. Re-run the exact same command; the cursor remains at seq {}.",
                        idle_timeout, resume,
                    );
                } else {
                    eprintln!(
                        "No message for {}s — the turn may be stalled. Re-run cowchat with the same identity and options, using --since-seq {}.",
                        idle_timeout, resume,
                    );
                }
                std::process::exit(2);
            }

            match matched? {
                Some(msg) => {
                    let mut output_file = open_output_file(output.as_ref(), cursor_file.as_ref())?;
                    let mut emitted = 0usize;
                    let mut saw_conversation_end = false;
                    let checkpoint_seq = if *drain {
                        // Re-pull every unread row through a fixed tip in bounded
                        // pages. Validate every sequence, but emit only matching
                        // peer chat. The checkpoint represents all evaluated rows.
                        let captured_tip = client.room_tip(&room_id).await?;
                        let advanced = latest_seq.load(std::sync::atomic::Ordering::Relaxed);
                        let mut drain_floor = if advanced == i64::MIN {
                            resolved_since_seq.unwrap_or_else(|| msg.seq.saturating_sub(1))
                        } else {
                            advanced
                        };
                        while drain_floor < captured_tip {
                            let page = client
                                .get_contiguous_history_page(
                                    &room_id,
                                    drain_floor,
                                    captured_tip,
                                    500,
                                )
                                .await?;
                            for message in &page {
                                let row_type =
                                    message.metadata.get("type").and_then(|v| v.as_str());
                                if row_type == Some("thinking")
                                    || row_type == Some("system")
                                    || client.is_self_message(message)
                                    || !matches(message)
                                {
                                    continue;
                                }
                                let rendered = if *text {
                                    format_message(message)
                                } else {
                                    serde_json::to_string(message)?
                                };
                                write_output_line(&mut output_file, &rendered)?;
                                emitted += 1;
                                saw_conversation_end |= message
                                    .metadata
                                    .get("kind")
                                    .and_then(|value| value.as_str())
                                    == Some(KIND_CONVERSATION_END);
                            }
                            drain_floor = page
                                .last()
                                .map(|message| message.seq)
                                .ok_or("contiguous drain page unexpectedly empty")?;
                        }
                        captured_tip
                    } else {
                        let rendered = if *text {
                            format_message(&msg)
                        } else {
                            serde_json::to_string(&msg)?
                        };
                        write_output_line(&mut output_file, &rendered)?;
                        emitted = 1;
                        saw_conversation_end =
                            msg.metadata.get("kind").and_then(|value| value.as_str())
                                == Some(KIND_CONVERSATION_END);
                        msg.seq
                    };
                    finish_output(&mut output_file)?;
                    if let Some(path) = output {
                        eprintln!("wrote {emitted} message(s) to {}", path.display());
                    }

                    // Advance the cursor to the highest seq we just processed —
                    // never our own sent seq, only what we received.
                    if let Some(path) = cursor_file {
                        write_cursor_atomic(path, &scope, checkpoint_seq).map_err(|error| {
                            Box::<dyn std::error::Error>::from(format!(
                                "failed to advance cursor file {}: {error}",
                                path.display()
                            ))
                        })?;
                    }
                    if *drain {
                        eprintln!("drained through seq {checkpoint_seq}");
                    }

                    // If any message in the batch ended the conversation, stop the
                    // loop (exit 3) instead of waiting for another turn.
                    if saw_conversation_end {
                        eprintln!("Peer ended the conversation.");
                        std::process::exit(3);
                    }
                }
                None => {
                    eprintln!("Timed out after {}s waiting for a message", timeout);
                    std::process::exit(1);
                }
            }
        }

        Commands::Monitor { room, json } => {
            let client = connect(&cli).await?;
            let room_secret = resolve_room_secret(&cli);

            // Join room if specified to receive its events
            if let Some(room_id) = room {
                let _ = client.join_room(room_id).await;
            }

            let mut events = client.subscribe();
            println!("Monitoring events (Ctrl+C to stop)...");
            while let Ok(event) = events.recv().await {
                if *json {
                    println!(
                        "{}",
                        serde_json::to_string(&event.frame).unwrap_or_default()
                    );
                } else {
                    print_event(&event.frame, room_secret.as_deref());
                }
            }
        }

        Commands::Shell { room } => {
            run_shell(&cli, room).await?;
        }

        Commands::Keygen => {
            // Purely local — no server connection.
            let key = cowchat_core::crypto::generate_secret();
            println!("{key}");
            eprintln!("# Set the SAME value on every agent in the group:");
            eprintln!("#   export COWCHAT_ROOM_KEY={key}");
        }

        Commands::Status => {
            let client = connect(&cli).await?;
            let agents = client.list_agents(None).await?;
            let rooms = client.list_rooms(None).await?;
            println!("Cowchat Server Status");
            println!("  Connected agents: {}", agents.len());
            println!("  Active rooms: {}", rooms.len());
            println!();
            if !agents.is_empty() {
                println!("Agents:");
                for agent in &agents {
                    println!("  - {} ({})", agent.name, agent.agent_id);
                }
            }
        }

        Commands::Export {
            room,
            format,
            since_seq,
            limit,
            include_thinking,
            output,
        } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;

            // Pull history. Use a generous default cap; clients with very long
            // rooms can pass --limit to shrink. Default 1000 — agent rooms in
            // the wild rarely exceed this in a single review session.
            let cap = limit.unwrap_or(1000);
            let messages = client
                .get_history_filtered(&room_id, cap, None, None, *since_seq)
                .await?;

            let body = render_export(&messages, *format, *include_thinking, room);

            match output {
                Some(path) => {
                    std::fs::write(path, body)?;
                    eprintln!("wrote {} messages to {}", messages.len(), path.display());
                }
                None => {
                    print!("{}", body);
                }
            }
        }

        Commands::Sub { action } => {
            let client = connect(&cli).await?;
            match action {
                SubAction::Create {
                    room,
                    url,
                    secret,
                    kinds,
                    only_from,
                    not_from,
                    exclude_thinking,
                    since_seq,
                } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let since: Option<i64> = match since_seq.to_lowercase().as_str() {
                        "tip" | "auto" => None,
                        s => Some(s.parse::<i64>().map_err(|e| {
                            Box::<dyn std::error::Error>::from(format!(
                                "--since-seq must be an integer, 'tip', or 'auto': {}",
                                e
                            ))
                        })?),
                    };
                    let sub = client
                        .create_subscription(
                            &room_id,
                            url,
                            secret,
                            kinds.clone(),
                            only_from.as_deref(),
                            not_from.as_deref(),
                            *exclude_thinking,
                            since,
                        )
                        .await?;
                    println!("{}", serde_json::to_string_pretty(&sub)?);
                }
                SubAction::List { room } => {
                    let room_id_opt: Option<String> = match room {
                        Some(r) => Some(resolve_room_id(&client, r).await?),
                        None => None,
                    };
                    let subs = client.list_subscriptions(room_id_opt.as_deref()).await?;
                    if subs.is_empty() {
                        println!("No subscriptions.");
                    } else {
                        println!(
                            "{:<38} {:<38} {:<10} {:<10} {:<12} URL",
                            "ID", "ROOM", "STATUS", "FAILS", "LAST_DELIV"
                        );
                        println!("{}", "-".repeat(140));
                        for s in subs {
                            println!(
                                "{:<38} {:<38} {:<10} {:<10} #{:<11} {}",
                                s.subscription_id,
                                s.room_id,
                                s.status,
                                s.failure_count,
                                s.last_delivered_seq,
                                s.webhook_url
                            );
                        }
                    }
                }
                SubAction::Delete { subscription_id } => {
                    client.unsubscribe(subscription_id).await?;
                    println!("Deleted {}", subscription_id);
                }
                SubAction::Enable { subscription_id } => {
                    client.enable_subscription(subscription_id).await?;
                    println!("Enabled {}", subscription_id);
                }
            }
        }

        Commands::Vote { action } => {
            let client = connect(&cli).await?;
            match action {
                VoteAction::Create {
                    room,
                    title,
                    options,
                    description,
                    duration,
                } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    client.join_room(&room_id).await?;
                    let info = client
                        .create_vote(
                            &room_id,
                            title,
                            description.as_deref(),
                            options.clone(),
                            *duration,
                        )
                        .await?;
                    println!("Vote created: {} ({})", info.title, info.vote_id);
                    println!("  Room: {}", info.room_id);
                    println!("  Options:");
                    for (i, opt) in info.options.iter().enumerate() {
                        println!("    [{}] {}", i, opt);
                    }
                    if let Some(deadline) = info.closes_at {
                        println!("  Closes at: {}", deadline.format("%H:%M:%S"));
                    } else {
                        println!("  Closes when all {} members vote", info.eligible_voters);
                    }
                }
                VoteAction::Cast { vote_id, option } => {
                    let info = client.get_vote_status(vote_id).await?;
                    client.join_room(&info.room_id).await?;
                    let resp = client.cast_vote(vote_id, *option).await?;
                    let votes_cast = resp.get("votes_cast").and_then(|v| v.as_u64()).unwrap_or(0);
                    let eligible = resp
                        .get("eligible_voters")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    println!("Ballot cast ({}/{} votes in)", votes_cast, eligible);
                }
                VoteAction::Status { vote_id } => {
                    let info = client.get_vote_status(vote_id).await?;
                    println!("Vote: {} ({})", info.title, info.vote_id);
                    println!("  Status: {:?}", info.status);
                    println!("  Votes cast: {}/{}", info.votes_cast, info.eligible_voters);
                    if let Some(closes_at) = info.closes_at {
                        println!("  Closes at: {}", closes_at.format("%Y-%m-%d %H:%M:%S UTC"));
                    }
                    println!("  Options:");
                    for (i, opt) in info.options.iter().enumerate() {
                        println!("    [{}] {}", i, opt);
                    }
                    if let Some(tally) = info.tally {
                        println!("  Tally:");
                        for row in tally {
                            println!(
                                "    [{}] {}: {}",
                                row.option_index, row.option_text, row.count
                            );
                        }
                    }
                }
                VoteAction::History { room, limit } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let votes = client.list_votes(&room_id, *limit).await?;

                    if votes.is_empty() {
                        println!("No votes found for room {}", room_id);
                    } else {
                        println!("Votes for room {}:", room_id);
                        for vote in votes {
                            println!(
                                "- {} ({}) {:?} {}/{}",
                                vote.title,
                                vote.vote_id,
                                vote.status,
                                vote.votes_cast,
                                vote.eligible_voters
                            );
                        }
                    }
                }
            }
        }

        Commands::Election { action } => {
            let client = connect(&cli).await?;
            match action {
                ElectionAction::Start { room } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    client.join_room(&room_id).await?;
                    let resp = client.elect_leader(&room_id).await?;
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                ElectionAction::Decline { room } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    client.join_room(&room_id).await?;
                    let resp = client.decline_election(&room_id).await?;
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                ElectionAction::Decide { room, content } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    client.join_room(&room_id).await?;
                    let resp = client
                        .send_decision(&room_id, content, serde_json::json!({}))
                        .await?;
                    println!("Decision issued: {}", serde_json::to_string_pretty(&resp)?);
                }
            }
        }

        Commands::Presence {
            status,
            detail,
            progress,
        } => {
            let client = connect(&cli).await?;
            client
                .set_presence(status, detail.as_deref(), *progress)
                .await?;
            let mut msg = format!("Presence set to: {}", status);
            if let Some(p) = progress {
                msg.push_str(&format!(" ({}%)", p));
            }
            if let Some(d) = detail {
                msg.push_str(&format!(": {}", d));
            }
            println!("{}", msg);
        }

        Commands::Lantern { action } => {
            lantern_cmd(&cli, action).await?;
        }
    }

    Ok(())
}

/// Send a LANTERN envelope as a room message: validate, then post with a
/// `kind:"lantern"` metadata marker (content is encrypted by the client in
/// encrypted rooms). Prints the assigned seq; for opening verbs that seq IS the
/// thread id. Refuses to send a malformed envelope.
async fn lantern_send(
    cli: &Cli,
    room: &str,
    value: serde_json::Value,
    opening: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let errs = lantern::validate(&value);
    if !errs.is_empty() {
        return Err(format!("malformed LANTERN message:\n  - {}", errs.join("\n  - ")).into());
    }
    let client = connect(cli).await?;
    let room_id = resolve_room_id(&client, room).await?;
    client.join_room(&room_id).await?;
    let content = serde_json::to_string(&value)?;
    let msg = client
        .send_message_with_metadata(
            &room_id,
            &content,
            None,
            vec![],
            serde_json::json!({ "kind": lantern::LANTERN_KIND }),
        )
        .await?;
    let verb = value.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    if opening {
        println!("{verb} sent as seq {0} — thread id is {0}", msg.seq);
    } else {
        println!("{verb} sent as seq {}", msg.seq);
    }
    Ok(())
}

/// Load and decrypt a room's history for client-side LANTERN reconstruction.
async fn lantern_history(
    cli: &Cli,
    room: &str,
) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error>> {
    let client = connect(cli).await?;
    let room_id = resolve_room_id(&client, room).await?;
    Ok(client
        .get_history_filtered(&room_id, 1000, None, None, None)
        .await?)
}

async fn lantern_cmd(cli: &Cli, action: &LanternAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        LanternAction::Hello {
            room,
            provider,
            model,
            role,
            capabilities,
        } => {
            let caps = capabilities
                .iter()
                .map(|c| {
                    let (name, fby) = c.split_once('=').unwrap_or((c.as_str(), ""));
                    lantern::Capability {
                        name: name.trim().to_string(),
                        falsifiable_by: (!fby.trim().is_empty()).then(|| fby.trim().to_string()),
                    }
                })
                .collect();
            let hello = lantern::Hello::new(
                &cli.name,
                provider.clone(),
                model.clone(),
                role.clone(),
                caps,
            );
            lantern_send(cli, room, serde_json::to_value(hello)?, false).await?;
        }
        LanternAction::Probe {
            room,
            question,
            intent,
        } => {
            let env = lantern::Envelope::new(
                "PROBE",
                &cli.name,
                None,
                None,
                intent.clone(),
                serde_json::json!({ "question": question }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, true).await?;
        }
        LanternAction::Assert {
            room,
            claim,
            confidence,
            falsifiable_by,
            intent,
        } => {
            let mut body = serde_json::json!({ "claim": claim, "falsifiable_by": falsifiable_by });
            if let Some(c) = confidence {
                body["confidence"] = serde_json::json!(c);
            }
            let env = lantern::Envelope::new("ASSERT", &cli.name, None, None, intent.clone(), body);
            lantern_send(cli, room, serde_json::to_value(env)?, true).await?;
        }
        LanternAction::Challenge {
            room,
            thread,
            target_seq,
            counter_claim,
            confidence,
            test,
        } => {
            let env = lantern::Envelope::new(
                "CHALLENGE",
                &cli.name,
                Some(*thread),
                Some(*target_seq),
                None,
                serde_json::json!({ "target_seq": target_seq, "counter_claim": counter_claim, "confidence": confidence, "test": test }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Resolve {
            room,
            thread,
            observation,
            basis,
        } => {
            let env = lantern::Envelope::new(
                "RESOLVE",
                &cli.name,
                Some(*thread),
                None,
                None,
                serde_json::json!({ "observation": observation, "basis": basis }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Fuse {
            room,
            thread,
            synthesis,
            state_delta,
            split,
            outcomes,
        } => {
            let mut body = serde_json::json!({ "synthesis": synthesis, "split": split });
            if let Some(path) = state_delta {
                let raw = std::fs::read_to_string(path)?;
                body["shared_state_delta"] = serde_json::from_str(&raw)?;
            }
            if !outcomes.is_empty() {
                let mut map = serde_json::Map::new();
                for o in outcomes {
                    let (seq, verdict) = o.split_once('=').ok_or_else(|| {
                        format!("--outcome must be <seq>=<true|false>, got `{o}`")
                    })?;
                    map.insert(
                        seq.trim().to_string(),
                        serde_json::json!(verdict.trim().parse::<bool>()?),
                    );
                }
                body["outcomes"] = serde_json::Value::Object(map);
            }
            let env = lantern::Envelope::new("FUSE", &cli.name, Some(*thread), None, None, body);
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Sync {
            room,
            thread,
            state_hash,
            diff,
        } => {
            let mut body = serde_json::json!({});
            if let Some(h) = state_hash {
                body["state_hash"] = serde_json::json!(h);
            }
            if let Some(path) = diff {
                body["diff"] = serde_json::from_str(&std::fs::read_to_string(path)?)?;
            }
            let env = lantern::Envelope::new("SYNC", &cli.name, *thread, None, None, body);
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Spark {
            room,
            seed,
            why_now,
            smallest_test,
        } => {
            let env = lantern::Envelope::new(
                "SPARK",
                &cli.name,
                None,
                None,
                None,
                serde_json::json!({ "seed": seed, "why_now": why_now, "smallest_test": smallest_test }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Harvest {
            room,
            spark_seq,
            becomes,
        } => {
            let env = lantern::Envelope::new(
                "HARVEST",
                &cli.name,
                None,
                Some(*spark_seq),
                None,
                serde_json::json!({ "spark_seq": spark_seq, "becomes": becomes }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Bury {
            room,
            spark_seq,
            reason,
        } => {
            let env = lantern::Envelope::new(
                "BURY",
                &cli.name,
                None,
                Some(*spark_seq),
                None,
                serde_json::json!({ "spark_seq": spark_seq, "reason": reason }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Threads { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            // Provenance first: who has announced themselves via HELLO.
            if !rec.hellos.is_empty() {
                println!("Participants (HELLO, self-attested):");
                for (seq, h) in &rec.hellos {
                    let who = [h.provider.as_deref(), h.model.as_deref(), h.role.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" / ");
                    let caps = h
                        .capabilities
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "  #{seq} {} — {} [{}]",
                        h.agent_name,
                        if who.is_empty() { "?".into() } else { who },
                        caps
                    );
                }
                println!();
            }
            if rec.threads.is_empty() {
                println!("No LANTERN threads in this room.");
                return Ok(());
            }
            println!("{:<8} {:<9} {:<8} HEADLINE", "THREAD", "STATE", "MSGS");
            for t in &rec.threads {
                let state = match t.state {
                    lantern::ThreadState::Open => "open",
                    lantern::ThreadState::Resolved => "resolved",
                    lantern::ThreadState::Fused => "fused",
                };
                println!(
                    "{:<8} {:<9} {:<8} {}",
                    t.id,
                    state,
                    t.messages.len(),
                    t.headline()
                );
            }
            // Surface the REFRACTION-due nudge (every third FUSE across the room).
            if rec.fuse_count > 0 && rec.fuse_count.is_multiple_of(3) {
                eprintln!(
                    "note: {} FUSEs — the next FUSE is REFRACTION-due (non-author picks the lens).",
                    rec.fuse_count
                );
            }
        }
        LanternAction::Show { room, thread } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            match rec.threads.iter().find(|t| t.id == *thread) {
                None => println!("No thread {thread} in this room."),
                Some(t) => {
                    for m in &t.messages {
                        let intent = m
                            .intent
                            .as_deref()
                            .map(|i| format!("  // {i}"))
                            .unwrap_or_default();
                        println!("#{} {} {}{}", m.seq, m.from, m.verb, intent);
                        println!("    {}", serde_json::to_string(&m.body).unwrap_or_default());
                    }
                }
            }
        }
        LanternAction::State { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            if rec.shared_state.is_empty() {
                println!("No committed shared state (no FUSE with a shared_state_delta yet).");
                return Ok(());
            }
            println!("Committed shared-state deltas (in FUSE order):");
            for (i, d) in rec.shared_state.iter().enumerate() {
                println!(
                    "  {}. {}",
                    i + 1,
                    serde_json::to_string(d).unwrap_or_default()
                );
            }
        }
        LanternAction::Calibration { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            let cal = lantern::calibration(&rec);
            if cal.per_agent.is_empty() {
                println!("No calibration data yet (needs FUSEd threads with tool/artifact/human basis and recorded outcomes).");
                return Ok(());
            }
            println!("{:<20} {:<10} CLAIMS", "AGENT", "MEAN LOSS");
            for (agent, (_, n)) in &cal.per_agent {
                let mean = cal.mean(agent).unwrap_or(0.0);
                println!("{:<20} {:<10.4} {}", agent, mean, n);
            }
            println!("(lower loss is better; diagnostic only, not authority)");
        }
        LanternAction::Validate { path } => {
            let raw = if path == "-" {
                use std::io::Read;
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(path)?
            };
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            let errs = lantern::validate(&value);
            if errs.is_empty() {
                println!("valid");
            } else {
                println!("invalid:");
                for e in &errs {
                    println!("  - {e}");
                }
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn print_event(frame: &cowchat_core::Frame, room_secret: Option<&[u8]>) {
    match frame.frame_type {
        FrameType::MessageReceived => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[message] #{} {}: {}", room, agent, content);
        }
        FrameType::Mention => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "[mention] from #{}: {:?}",
                room,
                frame.payload.get("message")
            );
        }
        FrameType::AgentJoined => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[join] {} joined #{}", agent, room);
        }
        FrameType::AgentLeft => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leave] {} left #{}", agent, room);
        }
        FrameType::RoomCreated => {
            let name = frame
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let ephemeral = frame
                .payload
                .get("ephemeral")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tag = if ephemeral { " (ephemeral)" } else { "" };
            println!("[room+] created #{}{}", name, tag);
        }
        FrameType::RoomUpdated => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let name = frame
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[room~] renamed #{} to #{}", room, name);
        }
        FrameType::RoomDestroyed => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[room-] destroyed #{}", room);
        }
        FrameType::VoteCreated => {
            let title = frame
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let vote_id = frame
                .payload
                .get("vote_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[vote] New vote in #{}: \"{}\" ({})", room, title, vote_id);
        }
        FrameType::VoteResult => {
            let title = frame
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[vote-result] #{} \"{}\":", room, title);
            if let Some(tally) = frame.payload.get("tally").and_then(|v| v.as_array()) {
                for entry in tally {
                    let text = entry
                        .get("option_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let count = entry.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                    println!("  {} : {} votes", text, count);
                }
            }
        }
        FrameType::ElectionStarted => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "[election] Election started in #{} (2s opt-out window)",
                room
            );
        }
        FrameType::LeaderElected => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let name = frame
                .payload
                .get("leader_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leader] {} elected leader of #{}", name, room);
        }
        FrameType::LeaderCleared => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let reason = frame
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leader-] Leadership cleared in #{}: {}", room, reason);
        }
        FrameType::DecisionMade => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let leader = frame
                .payload
                .get("leader_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[decision] #{} {} decides: {}", room, leader, content);
        }
        FrameType::PresenceUpdate => {
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let status = frame
                .payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let progress = frame
                .payload
                .get("progress")
                .and_then(|v| v.as_u64())
                .map(|p| format!(" ({}%)", p))
                .unwrap_or_default();
            let detail = frame
                .payload
                .get("status_detail")
                .and_then(|v| v.as_str())
                .map(|d| format!(": {}", d))
                .unwrap_or_default();
            println!(
                "[presence] {} is now {}{}{}",
                agent, status, progress, detail
            );
        }
        FrameType::Thinking => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[thinking] #{} {}: {}", room, agent, content);
        }
        FrameType::TurnChanged => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let holder = frame
                .payload
                .get("current_turn_holder")
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");
            let reason = frame
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[turn] #{} -> {} ({})", room, holder, reason);
        }
        _ => {
            println!("[{:?}] {:?}", frame.frame_type, frame.payload);
        }
    }
}

#[cfg(test)]
mod room_key_tests {
    use super::{
        command_opens_connection, command_requires_stable_agent_id, reject_output_cursor_alias,
        resolve_room_key, write_cursor_atomic, Cli, Commands, CursorScope, RoomAction,
    };
    use clap::Parser;

    fn cursor_seq(path: &std::path::Path, scope: &CursorScope, tip: i64) -> i64 {
        super::read_cursor(path, scope, tip, false)
            .unwrap()
            .unwrap()
            .seq
    }

    #[test]
    fn room_key_resolution_precedence() {
        std::env::remove_var("COWCHAT_ROOM_KEY");
        assert_eq!(resolve_room_key(None), None);

        std::env::set_var("COWCHAT_ROOM_KEY", "");
        assert_eq!(resolve_room_key(None), None, "empty env = unset");

        std::env::set_var("COWCHAT_ROOM_KEY", "from-env");
        assert_eq!(resolve_room_key(None).as_deref(), Some("from-env"));

        assert_eq!(
            resolve_room_key(Some("flag".into())).as_deref(),
            Some("flag")
        );

        std::env::remove_var("COWCHAT_ROOM_KEY");
    }

    #[test]
    fn destroy_command_parses_explicit_confirmation() {
        let cli = Cli::try_parse_from([
            "cowchat",
            "--agent-id",
            "creator",
            "rooms",
            "destroy",
            "room-id",
            "--yes",
        ])
        .unwrap();
        match cli.command {
            Commands::Rooms {
                action: RoomAction::Destroy { room, yes },
            } => {
                assert_eq!(room, "room-id");
                assert!(yes);
            }
            _ => panic!("expected rooms destroy"),
        }
    }

    #[test]
    fn create_command_preserves_global_agent_name() {
        let cli = Cli::try_parse_from([
            "cowchat",
            "--name",
            "agent-a",
            "--agent-id",
            "stable-agent-a",
            "rooms",
            "create",
            "war-room",
            "--public",
        ])
        .unwrap();
        assert_eq!(cli.name, "agent-a");
        match cli.command {
            Commands::Rooms {
                action:
                    RoomAction::Create {
                        room_name, public, ..
                    },
            } => {
                assert_eq!(room_name, "war-room");
                assert!(public);
            }
            _ => panic!("expected rooms create"),
        }
    }

    #[test]
    fn rename_command_preserves_global_agent_name_and_parses_room_name() {
        let cli = Cli::try_parse_from([
            "cowchat",
            "--name",
            "agent-a",
            "--agent-id",
            "creator",
            "rooms",
            "rename",
            "old-name",
            "New Room",
        ])
        .unwrap();
        assert_eq!(cli.name, "agent-a");
        match cli.command {
            Commands::Rooms {
                action: RoomAction::Rename { room, new_name },
            } => {
                assert_eq!(room, "old-name");
                assert_eq!(new_name, "New Room");
            }
            _ => panic!("expected rooms rename"),
        }
    }

    #[test]
    fn scoped_cursor_rejects_mismatch_and_room_reset() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("cursor");
        let scope = CursorScope {
            endpoint: "tcp:127.0.0.1:9229".into(),
            room_id: "room-a".into(),
            agent_id: "agent-a".into(),
        };
        write_cursor_atomic(&path, &scope, 5).unwrap();
        assert_eq!(cursor_seq(&path, &scope, 5), 5);

        for mismatched in [
            CursorScope {
                endpoint: "tcp:other".into(),
                ..scope.clone()
            },
            CursorScope {
                room_id: "room-b".into(),
                ..scope.clone()
            },
            CursorScope {
                agent_id: "agent-b".into(),
                ..scope.clone()
            },
        ] {
            let mismatch = super::read_cursor(&path, &mismatched, 5, false)
                .unwrap_err()
                .to_string();
            assert!(mismatch.contains("cursor scope mismatch"));
        }

        let reset = super::read_cursor(&path, &scope, 4, false)
            .unwrap_err()
            .to_string();
        assert!(reset.contains("ahead of room tip"));
        assert!(reset.contains("reset"));
    }

    #[test]
    fn unscoped_legacy_cursor_requires_explicit_import_and_is_upgraded() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("cursor");
        std::fs::write(&path, "3").unwrap();
        let scope = CursorScope {
            endpoint: "tcp:test".into(),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };
        let rejected = super::read_cursor(&path, &scope, 3, false)
            .unwrap_err()
            .to_string();
        assert!(rejected.contains("--import-legacy-cursor"));
        let loaded = super::read_cursor(&path, &scope, 3, true).unwrap().unwrap();
        assert_eq!(loaded.seq, 3);
        assert!(loaded.needs_upgrade);
        assert!(loaded.unscoped_legacy);
        assert!(write_cursor_atomic(&path, &scope, 3).is_err());
        super::upgrade_loaded_cursor(&path, &scope, loaded).unwrap();
        let upgraded = super::read_cursor(&path, &scope, 3, false)
            .unwrap()
            .unwrap();
        assert!(!upgraded.needs_upgrade);

        std::fs::write(&path, "3").unwrap();
        assert!(super::read_cursor(&path, &scope, 2, true).is_err());
        std::fs::write(&path, "-1").unwrap();
        assert!(super::read_cursor(&path, &scope, 3, true).is_err());
        assert!(write_cursor_atomic(&path, &scope, -1).is_err());
    }

    #[test]
    fn version_one_scoped_cursor_migrates_without_legacy_opt_in() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("cursor");
        let raw_endpoint = "url:wss://user:secret@example.test/ws?token=hidden";
        let scope = CursorScope {
            endpoint: super::endpoint_fingerprint(raw_endpoint),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1,
                "endpoint": raw_endpoint,
                "room_id": "lobby",
                "agent_id": "agent-a",
                "seq": 2
            })
            .to_string(),
        )
        .unwrap();
        let loaded = super::read_cursor(&path, &scope, 2, false)
            .unwrap()
            .unwrap();
        assert!(loaded.needs_upgrade);
        assert!(!loaded.unscoped_legacy);
        super::upgrade_loaded_cursor(&path, &scope, loaded).unwrap();
        let encoded = std::fs::read_to_string(&path).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("token=hidden"));
        assert!(encoded.contains("sha256:"));
    }

    #[test]
    fn concurrent_cursor_writers_cannot_regress_the_checkpoint() {
        assert_eq!(
            super::cursor_parent(std::path::Path::new("cursor")),
            std::path::Path::new(".")
        );
        let temp = tempfile::TempDir::new().unwrap();
        let path = std::sync::Arc::new(temp.path().join("cursor"));
        let scope = std::sync::Arc::new(CursorScope {
            endpoint: "tcp:test".into(),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        });
        write_cursor_atomic(path.as_ref(), scope.as_ref(), 0).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(33));
        let mut writers = Vec::new();
        for seq in 1..=32 {
            let path = path.clone();
            let scope = scope.clone();
            let barrier = barrier.clone();
            writers.push(std::thread::spawn(move || {
                barrier.wait();
                let _ = write_cursor_atomic(path.as_ref(), scope.as_ref(), seq);
            }));
        }
        barrier.wait();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(cursor_seq(path.as_ref(), scope.as_ref(), 32), 32);
    }

    #[cfg(unix)]
    #[test]
    fn cursor_write_does_not_follow_predictable_or_target_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().unwrap();
        let cursor = temp.path().join("cursor");
        let victim = temp.path().join("victim");
        std::fs::write(&victim, "safe").unwrap();
        let old_predictable = temp
            .path()
            .join(format!(".cursor.{}.tmp", std::process::id()));
        symlink(&victim, &old_predictable).unwrap();
        let scope = CursorScope {
            endpoint: "tcp:test".into(),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };

        write_cursor_atomic(&cursor, &scope, 1).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");
        assert_eq!(cursor_seq(&cursor, &scope, 1), 1);

        std::fs::remove_file(&cursor).unwrap();
        symlink(&victim, &cursor).unwrap();
        assert!(write_cursor_atomic(&cursor, &scope, 2).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");
    }

    #[cfg(unix)]
    #[test]
    fn output_and_cursor_symlink_alias_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().unwrap();
        let cursor = temp.path().join("cursor");
        let output = temp.path().join("output-link");
        std::fs::write(&cursor, "0").unwrap();
        symlink(&cursor, &output).unwrap();
        assert!(reject_output_cursor_alias(Some(&output), Some(&cursor)).is_err());
        assert!(reject_output_cursor_alias(Some(&cursor), Some(&cursor)).is_err());

        std::fs::remove_file(&output).unwrap();
        std::fs::remove_file(&cursor).unwrap();
        symlink(&cursor, &output).unwrap();
        assert!(
            reject_output_cursor_alias(Some(&output), Some(&cursor)).is_err(),
            "a dangling output symlink to a missing cursor is still an alias"
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_and_cursor_hardlink_alias_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let cursor = temp.path().join("cursor");
        let output = temp.path().join("output-hardlink");
        std::fs::write(&cursor, "cursor-data").unwrap();
        std::fs::hard_link(&cursor, &output).unwrap();
        assert!(reject_output_cursor_alias(Some(&output), Some(&cursor)).is_err());
        assert!(super::open_output_file(Some(&output), Some(&cursor)).is_err());
        assert_eq!(std::fs::read_to_string(&cursor).unwrap(), "cursor-data");
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_cursor_is_rejected_even_without_an_output_path() {
        let temp = tempfile::TempDir::new().unwrap();
        let cursor = temp.path().join("cursor");
        let alias = temp.path().join("cursor-backup");
        let scope = CursorScope {
            endpoint: "sha256:test".into(),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };
        write_cursor_atomic(&cursor, &scope, 1).unwrap();
        std::fs::hard_link(&cursor, &alias).unwrap();
        assert!(super::read_cursor(&cursor, &scope, 1, false).is_err());
        assert!(write_cursor_atomic(&cursor, &scope, 2).is_err());
        let state: super::CursorState =
            serde_json::from_str(&std::fs::read_to_string(&alias).unwrap()).unwrap();
        assert_eq!(state.seq, 1);
    }

    #[cfg(unix)]
    #[test]
    fn cursor_files_are_owner_only_and_endpoint_secrets_are_not_persisted() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("cursor");
        let secret_endpoint = "url:wss://alice:password@example.test/ws?token=secret";
        let scope = CursorScope {
            endpoint: super::endpoint_fingerprint(secret_endpoint),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };
        write_cursor_atomic(&path, &scope, 1).unwrap();
        let encoded = std::fs::read_to_string(&path).unwrap();
        assert!(!encoded.contains("alice"));
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("token=secret"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::read_cursor(&path, &scope, 1, false).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn cursor_rejects_non_regular_files_without_changing_directory_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new().unwrap();
        let directory = temp.path().join("not-a-cursor");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let scope = CursorScope {
            endpoint: "sha256:test".into(),
            room_id: "lobby".into(),
            agent_id: "agent-a".into(),
        };
        assert!(super::read_cursor(&directory, &scope, 0, false).is_err());
        assert!(write_cursor_atomic(&directory, &scope, 0).is_err());
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn missing_case_only_output_cursor_alias_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("Progress.JSON");
        let cursor = temp.path().join("progress.json");
        assert!(reject_output_cursor_alias(Some(&output), Some(&cursor)).is_err());
    }

    #[test]
    fn offline_keygen_does_not_require_agent_identity() {
        let cli = Cli::try_parse_from(["cowchat", "--name", "reporter", "keygen"]).unwrap();
        assert!(!command_opens_connection(&cli.command));
        assert!(!command_requires_stable_agent_id(&cli.command));
    }

    #[test]
    fn unrecoverable_event_lag_fails_closed_instead_of_reconnecting_past_it() {
        assert!(!super::is_retryable_wait_error(
            &cowchat_client::ClientError::EventStreamLagged { skipped: 1 }
        ));
    }
}
